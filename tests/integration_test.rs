use minimq::{consts, MqttClient, Property, QoS};

use nb;
use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::net::{self, TcpStream};

use embedded_nal::{self, IpAddr, Ipv4Addr, SocketAddr};

struct Clock {}

impl embedded_time::Clock for Clock {
    type T = u64;

    const SCALING_FACTOR: embedded_time::fraction::Fraction =
        embedded_time::fraction::Fraction::new(1, 1);

    fn try_now(&self) -> Result<embedded_time::Instant<Self>, embedded_time::clock::Error> {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        Ok(embedded_time::Instant::new(time.as_secs()))
    }
}

struct StandardStack {
    stream: RefCell<Option<TcpStream>>,
    mode: RefCell<embedded_nal::Mode>,
}

impl StandardStack {
    pub fn new() -> StandardStack {
        StandardStack {
            stream: RefCell::new(None),
            mode: RefCell::new(embedded_nal::Mode::Blocking),
        }
    }
}

impl embedded_nal::TcpStack for StandardStack {
    type Error = io::Error;

    type TcpSocket = ();

    fn open(&self, mode: embedded_nal::Mode) -> Result<Self::TcpSocket, Self::Error> {
        self.mode.replace(mode);
        Ok(())
    }

    fn connect(
        &self,
        _socket: Self::TcpSocket,
        remote: SocketAddr,
    ) -> Result<Self::TcpSocket, Self::Error> {
        let ip = match remote.ip() {
            IpAddr::V4(addr) => net::IpAddr::V4(net::Ipv4Addr::new(
                addr.octets()[0],
                addr.octets()[1],
                addr.octets()[2],
                addr.octets()[3],
            )),
            IpAddr::V6(addr) => net::IpAddr::V6(net::Ipv6Addr::new(
                addr.segments()[0],
                addr.segments()[1],
                addr.segments()[2],
                addr.segments()[3],
                addr.segments()[4],
                addr.segments()[5],
                addr.segments()[6],
                addr.segments()[7],
            )),
        };

        let remote = net::SocketAddr::new(ip, remote.port());

        let stream = TcpStream::connect(remote).unwrap();

        match *self.mode.borrow() {
            embedded_nal::Mode::NonBlocking => stream.set_nonblocking(true)?,
            embedded_nal::Mode::Blocking => stream.set_nonblocking(false)?,
            embedded_nal::Mode::Timeout(t) => {
                stream.set_read_timeout(Some(std::time::Duration::from_secs(t.into())))?;
                stream.set_write_timeout(Some(std::time::Duration::from_secs(t.into())))?;
            }
        }
        self.stream.replace(Some(stream));

        Ok(())
    }

    fn is_connected(&self, _socket: &Self::TcpSocket) -> Result<bool, Self::Error> {
        Ok(self.stream.borrow().is_some())
    }

    fn write(
        &self,
        _socket: &mut Self::TcpSocket,
        buffer: &[u8],
    ) -> nb::Result<usize, Self::Error> {
        match &mut *self.stream.borrow_mut() {
            Some(stream) => match stream.write(buffer) {
                Ok(len) => Ok(len),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        Err(nb::Error::WouldBlock)
                    } else {
                        Err(nb::Error::Other(e))
                    }
                }
            },
            None => Ok(0),
        }
    }

    fn read(
        &self,
        _socket: &mut Self::TcpSocket,
        buffer: &mut [u8],
    ) -> nb::Result<usize, Self::Error> {
        match &mut *self.stream.borrow_mut() {
            Some(stream) => match stream.read(buffer) {
                Ok(len) => Ok(len),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        Err(nb::Error::WouldBlock)
                    } else {
                        Err(nb::Error::Other(e))
                    }
                }
            },
            None => Ok(0),
        }
    }

    fn close(&self, _socket: Self::TcpSocket) -> Result<(), Self::Error> {
        self.stream.replace(None).unwrap();

        Ok(())
    }
}

#[test]
fn main() -> std::io::Result<()> {
    env_logger::init();

    let stack = StandardStack::new();
    let localhost = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let my_clock = Clock {};
    let mut client =
        MqttClient::<consts::U256, _, _>::new(localhost, "IntegrationTest", stack, my_clock)
            .unwrap();

    let mut published = false;
    let mut subscribed = false;

    loop {
        client
            .poll(|client, topic, payload, properties| {
                println!("{} < {}", topic, core::str::from_utf8(payload).unwrap());

                for property in properties {
                    match property {
                        Property::ResponseTopic(topic) => client
                            .publish(topic, "Pong".as_bytes(), QoS::AtMostOnce, &[])
                            .unwrap(),
                        _ => {}
                    };
                }

                if topic == "response" {
                    std::process::exit(0);
                }
            })
            .unwrap();

        if !subscribed {
            if client.is_connected() {
                client.subscribe("response", &[]).unwrap();
                client.subscribe("request", &[]).unwrap();
                subscribed = true;
            }
        } else {
            if client.subscriptions_pending() == false {
                if !published {
                    println!("PUBLISH request");
                    let properties = [Property::ResponseTopic("response")];
                    client
                        .publish("request", "Ping".as_bytes(), QoS::AtMostOnce, &properties)
                        .unwrap();

                    published = true;
                }
            }
        }
    }
}
