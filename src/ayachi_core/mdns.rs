// Made by Han_feng

use super::{HOSTNAME, NAME, PORT};
use core::cell::Cell;
use core::net::Ipv4Addr;
use edge_mdns::buf::VecBufAccess;
use edge_mdns::domain::base::Ttl;
use edge_mdns::host::{Host, Service, ServiceAnswers};
use edge_mdns::{HostAnswer, HostAnswers, HostAnswersMdnsHandler, MdnsError};
use edge_nal::UdpSplit;
use edge_nal_embassy::UdpBuffers;
use embassy_net::{Ipv6Address, Stack};
use embassy_sync::blocking_mutex::CriticalSectionMutex;
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::signal::Signal;
use esp_hal::__macro_implementation::static_cell::StaticCell;

// Structs
pub struct MdnsAnswers {
    ipv4: CriticalSectionMutex<Cell<Ipv4Addr>>,
    signal: Signal<CriticalSectionRawMutex, ()>,
}

// Static
static MDNS_ANSWERS: StaticCell<MdnsAnswers> = StaticCell::new();

// Impls
impl MdnsAnswers {
    pub fn init() -> &'static Self {
        MDNS_ANSWERS.init(MdnsAnswers {
            ipv4: CriticalSectionMutex::new(Cell::new(Ipv4Addr::UNSPECIFIED)),
            signal: Signal::new(),
        })
    }

    pub fn update_ipv4(&self, ipv4: Ipv4Addr) {
        self.ipv4.lock(|ip| ip.set(ipv4));
        self.signal.signal(())
    }
}

impl HostAnswers for &MdnsAnswers {
    fn visit<F, E>(&self, f: F) -> Result<(), E>
    where
        F: FnMut(HostAnswer) -> Result<(), E>,
        E: From<MdnsError>
    {
        static SERVICE: Service<'static> = Service {
            name: NAME,
            priority: 0,
            weight: 0,
            service: "_http",
            protocol: "_tcp",
            port: PORT,
            service_subtypes: &[],
            txt_kvs: &[]
        };

        let host = Host {
            hostname: HOSTNAME,
            ipv4: self.ipv4.lock(|ip| ip.get()),
            ipv6: Ipv6Address::UNSPECIFIED,
            ttl: Ttl::from_secs(60),
        };

        ServiceAnswers::new(&host, &SERVICE).visit(f)
    }
}

// Tasks
#[embassy_executor::task]
pub async fn mdns_task(mdns_answers: &'static MdnsAnswers, stack: Stack<'static>) {
    static UDP_BUFFERS: StaticCell<UdpBuffers<1>> = StaticCell::new();

    let udp_adapter = edge_nal_embassy::Udp::new(stack, UDP_BUFFERS.init(UdpBuffers::new()));

    let mut socket = match edge_mdns::io::bind(
        &udp_adapter,
        edge_mdns::io::IPV4_DEFAULT_SOCKET,
        Some(Ipv4Addr::UNSPECIFIED),
        None,
    ).await {
        Ok(socket) => socket,
        Err(error) => {
            defmt::warn!("Failed to create socket for mdns, you may directly use the IP instead(message: {:?})", error);
            return;
        }
    };

    let (recv, send) = socket.split();

    let recv_buffer = VecBufAccess::<NoopRawMutex, 1500>::new();
    let send_buffer = VecBufAccess::<NoopRawMutex, 1500>::new();

    let mdns = edge_mdns::io::Mdns::new(
        Some(Ipv4Addr::UNSPECIFIED),
        None,
        recv, send,
        recv_buffer, send_buffer,
        esp_hal::rng::Rng::new(),
        &mdns_answers.signal
    );

    match mdns.run(HostAnswersMdnsHandler::new(mdns_answers)).await {
        Ok(()) => {},
        Err(error) => {
            defmt::warn!("An error occurred during mdns, you may directly use the IP instead(message: {:?})", error);
        }
    }
}