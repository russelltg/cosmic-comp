// SPDX-License-Identifier: GPL-3.0-only

// XXX Find secure way to pass socket to screen reader. Have cosmic-session or cosmic-com start it.

use crate::config::xkb_config_to_wl;
use crate::state::State;
use once_cell::sync::Lazy;
use reis::calloop::{EisListenerSource, EisRequestSource, EisRequestSourceEvent};
use reis::eis;
use reis::eis::device::DeviceType;
use reis::request::{DeviceCapability, EisRequest};
use rustix::fd::AsFd;
use smithay::backend::input::KeyState;
use smithay::utils::SerialCounter;
use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;
use xkbcommon::xkb;

pub static EI_SERIAL_COUNTER: SerialCounter = SerialCounter::new();

#[derive(Debug, Default)]
pub struct AtspiEiState {
    modifiers: smithay::input::keyboard::ModifiersState,
    // TODO: purge old instances
    keyboards: Vec<(eis::Context, eis::Device, eis::Keyboard)>,
}

impl AtspiEiState {
    pub fn input(
        &mut self,
        modifiers: &smithay::input::keyboard::ModifiersState,
        keysym: &smithay::input::keyboard::KeysymHandle,
        state: KeyState,
        time: u64,
    ) {
        let state = match state {
            KeyState::Pressed => eis::keyboard::KeyState::Press,
            KeyState::Released => eis::keyboard::KeyState::Released,
        };
        if &self.modifiers != modifiers {
            self.modifiers = *modifiers;
            for (_, _, keyboard) in &self.keyboards {
                keyboard.modifiers(
                    EI_SERIAL_COUNTER.next_serial().into(),
                    modifiers.serialized.depressed,
                    modifiers.serialized.locked,
                    modifiers.serialized.latched,
                    modifiers.serialized.layout_effective,
                );
            }
        }
        for (context, device, keyboard) in &self.keyboards {
            keyboard.key(keysym.raw_code().raw(), state);
            device.frame(EI_SERIAL_COUNTER.next_serial().into(), time);
            context.flush();
        }
    }
}

static SERVER_INTERFACES: Lazy<HashMap<&'static str, u32>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("ei_callback", 1);
    m.insert("ei_connection", 1);
    m.insert("ei_seat", 1);
    m.insert("ei_device", 1);
    m.insert("ei_pingpong", 1);
    m.insert("ei_keyboard", 1);
    m
});

pub fn listen_eis(handle: &calloop::LoopHandle<'static, State>) {
    let path = Path::new("/tmp/atspi-ei-kb.socket");
    std::fs::remove_file(&path); // XXX in use?
    let listener = eis::Listener::bind(&path).unwrap();
    let listener_source = EisListenerSource::new(listener);
    let handle_clone = handle.clone();
    handle
        .insert_source(listener_source, move |context, _, _| {
            let source = EisRequestSource::new(context, &SERVER_INTERFACES, 0);
            handle_clone
                .insert_source(source, |event, connected_state, state| {
                    match event {
                        Ok(EisRequestSourceEvent::Connected) => {
                            if connected_state.context_type
                                != reis::ei::handshake::ContextType::Receiver
                            {
                                return Ok(calloop::PostAction::Remove);
                            }
                            // TODO multiple seats
                            let seat = connected_state
                                .request_converter
                                .add_seat(Some("default"), &[DeviceCapability::Keyboard]);
                        }
                        Ok(EisRequestSourceEvent::Request(EisRequest::Disconnect)) => {
                            return Ok(calloop::PostAction::Remove);
                        }
                        Ok(EisRequestSourceEvent::Request(EisRequest::Bind(request))) => {
                            if connected_state.has_interface("ei_keyboard")
                                && request.capabilities & 2 << DeviceCapability::Keyboard as u64
                                    != 0
                            {
                                // TODO Handle keymap changes

                                let xkb_config = state.common.config.xkb_config();
                                let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                                let keymap = xkb::Keymap::new_from_names(
                                    &context,
                                    &xkb_config.rules,
                                    &xkb_config.model,
                                    &xkb_config.layout,
                                    &xkb_config.variant,
                                    xkb_config.options.clone(),
                                    xkb::KEYMAP_COMPILE_NO_FLAGS,
                                )
                                .unwrap();
                                let keymap_text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
                                // XXX make smithay SealedFile public?
                                // Share sealed file?
                                let fd = rustix::fs::memfd_create(
                                    "eis-keymap",
                                    rustix::fs::MemfdFlags::CLOEXEC,
                                )
                                .unwrap();
                                let mut file = std::fs::File::from(fd);
                                use std::io::Write;
                                file.write_all(keymap_text.as_bytes()).unwrap();

                                let device = connected_state.request_converter.add_device(
                                    &request.seat,
                                    Some("keyboard"),
                                    DeviceType::Virtual,
                                    &[DeviceCapability::Keyboard],
                                    |device| {
                                        let keyboard = device.interface::<eis::Keyboard>().unwrap();
                                        keyboard.keymap(
                                            eis::keyboard::KeymapType::Xkb,
                                            keymap_text.len() as _,
                                            file.as_fd(),
                                        );
                                    },
                                );
                                let keyboard = device.interface::<eis::Keyboard>().unwrap();

                                state.common.atspi_ei.keyboards.push((
                                    connected_state.context.clone(),
                                    device.device().clone(),
                                    keyboard,
                                ));
                            }
                        }
                        Ok(EisRequestSourceEvent::Request(request)) => {
                            // seat / keyboard / device release?
                        }
                        Ok(EisRequestSourceEvent::InvalidObject(_)) => {}
                        Err(_) => {
                            // TODO
                        }
                    }
                    connected_state.context.flush();
                    Ok(calloop::PostAction::Continue)
                })
                .unwrap();
            Ok(calloop::PostAction::Continue)
        })
        .unwrap();
}
