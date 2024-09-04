// SPDX-License-Identifier: GPL-3.0-only

use cosmic_protocols::atspi::v1::server::cosmic_atspi_manager_v1;

use smithay::reexports::wayland_server::{
    backend::GlobalId, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
};
use std::os::unix::{io::AsFd, net::UnixStream};

pub trait AtspiHandler {
    fn add_key_event_socket(&mut self, socket: UnixStream);
    fn add_key_grab(&mut self, mods: u32, virtual_mods: Vec<u32>, key: u32);
    fn remove_key_grab(&mut self, mods: u32, virtual_mods: Vec<u32>, key: u32);
}

#[derive(Debug)]
pub struct AtspiState {
    global: GlobalId,
}

impl AtspiState {
    pub fn new<D, F>(dh: &DisplayHandle, client_filter: F) -> AtspiState
    where
        D: GlobalDispatch<cosmic_atspi_manager_v1::CosmicAtspiManagerV1, AtspiGlobalData> + 'static,
        F: for<'a> Fn(&'a Client) -> bool + Send + Sync + 'static,
    {
        let global = dh.create_global::<D, cosmic_atspi_manager_v1::CosmicAtspiManagerV1, _>(
            1,
            AtspiGlobalData {
                filter: Box::new(client_filter),
            },
        );
        AtspiState { global }
    }
}

pub struct AtspiGlobalData {
    filter: Box<dyn for<'a> Fn(&'a Client) -> bool + Send + Sync>,
}

impl<D> GlobalDispatch<cosmic_atspi_manager_v1::CosmicAtspiManagerV1, AtspiGlobalData, D>
    for AtspiState
where
    D: GlobalDispatch<cosmic_atspi_manager_v1::CosmicAtspiManagerV1, AtspiGlobalData>
        + Dispatch<cosmic_atspi_manager_v1::CosmicAtspiManagerV1, ()>
        + AtspiHandler
        + 'static,
{
    fn bind(
        state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<cosmic_atspi_manager_v1::CosmicAtspiManagerV1>,
        _global_data: &AtspiGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let instance = data_init.init(resource, ());
        let (client_socket, server_socket) = UnixStream::pair().unwrap(); // XXX
        state.add_key_event_socket(server_socket);
        instance.key_events_eis(client_socket.as_fd());
    }

    fn can_view(client: Client, global_data: &AtspiGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<cosmic_atspi_manager_v1::CosmicAtspiManagerV1, (), D> for AtspiState
where
    D: Dispatch<cosmic_atspi_manager_v1::CosmicAtspiManagerV1, ()> + AtspiHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _obj: &cosmic_atspi_manager_v1::CosmicAtspiManagerV1,
        request: cosmic_atspi_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            cosmic_atspi_manager_v1::Request::AddKeyGrab {
                mods,
                virtual_mods,
                key,
            } => {
                let virtual_mods = virtual_mods
                    .chunks_exact(4)
                    .map(|x| u32::from_ne_bytes(<[u8; 4]>::try_from(x).unwrap()))
                    .collect();
                state.add_key_grab(mods, virtual_mods, key);
            }
            cosmic_atspi_manager_v1::Request::RemoveKeyGrab {
                mods,
                virtual_mods,
                key,
            } => {
                let virtual_mods = virtual_mods
                    .chunks_exact(4)
                    .map(|x| u32::from_ne_bytes(<[u8; 4]>::try_from(x).unwrap()))
                    .collect();
                state.remove_key_grab(mods, virtual_mods, key);
            }
            cosmic_atspi_manager_v1::Request::GrabKeyboard => {
                // TODO Set a grab like XWaylandKeyboardGrab
                // - Make sure there is only one such grab?
            }
            cosmic_atspi_manager_v1::Request::UngrabKeyboard => {}
            cosmic_atspi_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

macro_rules! delegate_atspi {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            cosmic_protocols::atspi::v1::server::cosmic_atspi_manager_v1::CosmicAtspiManagerV1: $crate::wayland::protocols::atspi::AtspiGlobalData
        ] => $crate::wayland::protocols::atspi::AtspiState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            cosmic_protocols::atspi::v1::server::cosmic_atspi_manager_v1::CosmicAtspiManagerV1: ()
        ] => $crate::wayland::protocols::atspi::AtspiState);
    };
}
pub(crate) use delegate_atspi;
