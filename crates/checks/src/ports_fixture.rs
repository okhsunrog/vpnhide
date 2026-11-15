use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, UdpSocket},
    time::Duration,
};
use vpnhide_diagnostic_model::{PortsFixtureSpec, PortsObservation, PortsSample, PortsVector};

const TIMEOUT: Duration = Duration::from_millis(350);

pub fn endpoint(vector: PortsVector, port: u16) -> SocketAddr {
    let ip = match vector {
        PortsVector::Tcp4 | PortsVector::Udp4 => IpAddr::V4(Ipv4Addr::LOCALHOST),
        PortsVector::Tcp6 | PortsVector::Udp6 => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    SocketAddr::new(ip, port)
}

pub fn observe(spec: &PortsFixtureSpec) -> PortsObservation {
    std::array::from_fn(|index| {
        let vector = PortsVector::ALL[index];
        let destination = endpoint(vector, spec.ports[index]);
        match exchange(vector, destination, &spec.challenge) {
            Ok(true) => PortsSample::Echo,
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => PortsSample::Refused,
            _ => PortsSample::Unavailable,
        }
    })
}

fn exchange(
    vector: PortsVector,
    destination: SocketAddr,
    challenge: &[u8; 16],
) -> io::Result<bool> {
    match vector {
        PortsVector::Tcp4 | PortsVector::Tcp6 => {
            let mut stream = TcpStream::connect_timeout(&destination, TIMEOUT)?;
            stream.set_read_timeout(Some(TIMEOUT))?;
            stream.set_write_timeout(Some(TIMEOUT))?;
            stream.write_all(challenge)?;
            let mut reply = [0; 16];
            stream.read_exact(&mut reply)?;
            Ok(&reply == challenge)
        }
        PortsVector::Udp4 | PortsVector::Udp6 => {
            let socket = UdpSocket::bind(endpoint(vector, 0))?;
            socket.connect(destination)?;
            socket.set_read_timeout(Some(TIMEOUT))?;
            socket.set_write_timeout(Some(TIMEOUT))?;
            socket.send(challenge)?;
            let mut reply = [0; 17];
            let len = socket.recv(&mut reply)?;
            Ok(&reply[..len] == challenge)
        }
    }
}
