// SPDX-License-Identifier: GPL-3.0-only

use once_cell::sync::Lazy;
use reis::{
    calloop::{ConnectedContextState, EisRequestSource, EisRequestSourceEvent},
    eis::{self, device::DeviceType},
    request::{DeviceCapability, EisRequest},
};
use smithay::{backend::input::KeyState, utils::SerialCounter};
use std::{
    collections::HashMap,
    os::unix::{io::AsFd, net::UnixStream},
};
use xkbcommon::xkb;

use crate::{
    state::State,
    wayland::protocols::atspi::{delegate_atspi, AtspiHandler},
};

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
            keyboard.key(keysym.raw_code().raw() - 8, state);
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

impl AtspiHandler for State {
    fn add_key_event_socket(&mut self, socket: UnixStream) {
        let context = eis::Context::new(socket).unwrap(); // XXX
        let source = EisRequestSource::new(context, &SERVER_INTERFACES, 0);
        self.common
            .event_loop_handle
            .insert_source(source, |event, connected_state, state| {
                Ok(handle_event(event, connected_state, state))
            })
            .unwrap(); // XXX
    }

    fn add_key_grab(&mut self, mods: u32, virtual_mods: Vec<u32>, key: u32) {
        tracing::error!("add_key_grab: {:?}", (mods, virtual_mods, key));
    }

    fn remove_key_grab(&mut self, mods: u32, virtual_mods: Vec<u32>, key: u32) {
        tracing::error!("remove_key_grab: {:?}", (mods, virtual_mods, key));
    }
}

fn handle_event(
    event: Result<EisRequestSourceEvent, reis::request::Error>,
    connected_state: &mut ConnectedContextState,
    state: &mut State,
) -> calloop::PostAction {
    match event {
        Ok(EisRequestSourceEvent::Connected) => {
            if connected_state.context_type != reis::ei::handshake::ContextType::Receiver {
                return calloop::PostAction::Remove;
            }
            // TODO multiple seats
            let _seat = connected_state
                .request_converter
                .add_seat(Some("default"), &[DeviceCapability::Keyboard]);
        }
        Ok(EisRequestSourceEvent::Request(EisRequest::Disconnect)) => {
            return calloop::PostAction::Remove;
        }
        Ok(EisRequestSourceEvent::Request(EisRequest::Bind(request))) => {
            if connected_state.has_interface("ei_keyboard")
                && request.capabilities & 2 << DeviceCapability::Keyboard as u64 != 0
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
                let fd = rustix::fs::memfd_create("eis-keymap", rustix::fs::MemfdFlags::CLOEXEC)
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
                device
                    .device()
                    .resumed(EI_SERIAL_COUNTER.next_serial().into());

                let keyboard = device.interface::<eis::Keyboard>().unwrap();

                keyboard.modifiers(
                    EI_SERIAL_COUNTER.next_serial().into(),
                    state.common.atspi_ei.modifiers.serialized.depressed,
                    state.common.atspi_ei.modifiers.serialized.locked,
                    state.common.atspi_ei.modifiers.serialized.latched,
                    state.common.atspi_ei.modifiers.serialized.layout_effective,
                );

                device
                    .device()
                    .start_emulating(EI_SERIAL_COUNTER.next_serial().into(), 0);

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
    calloop::PostAction::Continue
}

delegate_atspi!(State);
