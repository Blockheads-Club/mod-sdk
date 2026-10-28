//! The voice UDP fallback (`transport = "udp"`, or `auto` when in-band is too lossy): a non-blocking socket to the
//! server's voice port, opened when the driver first has a datagram for it and polled every frame.

use std::io::ErrorKind;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

/// Largest datagram read (voice datagrams are ~150 bytes).
pub const MAX_DATAGRAM: usize = 1500;
/// Most datagrams read in one frame (more wait for the next).
pub const MAX_PER_FRAME: usize = 64;

/// A socket to one server's voice port.
#[derive(Debug)]
pub struct UdpLink {
    sock: UdpSocket,
    server: SocketAddr,
}

impl UdpLink {
    /// Opens a socket towards `host` (the game server's address, without its port) and `port`.
    pub fn open(host: &str, port: u16) -> Result<UdpLink, String> {
        Self::try_open(host, port).map_err(|e| format!("{host}: {e}"))
    }

    fn try_open(host: &str, port: u16) -> std::io::Result<UdpLink> {
        let none = std::io::Error::from(ErrorKind::NotFound);
        let server = (host, port).to_socket_addrs()?.next().ok_or(none)?;
        let any: SocketAddr = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse().expect("literal");
        let sock = UdpSocket::bind(any)?;
        sock.set_nonblocking(true)?;
        Ok(UdpLink { sock, server })
    }

    /// The server's voice address.
    pub fn server(&self) -> SocketAddr {
        self.server
    }

    /// Sends one datagram (false when the OS refused it now).
    pub fn send(&self, datagram: &[u8]) -> bool {
        self.sock.send_to(datagram, self.server).is_ok()
    }

    /// Datagrams that arrived from the server since the last call (others are dropped).
    pub fn poll(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = [0u8; MAX_DATAGRAM];
        while out.len() < MAX_PER_FRAME {
            match self.sock.recv_from(&mut buf) {
                Ok((n, from)) if from == self.server => out.push(buf[..n].to_vec()),
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        out
    }
}

/// The host part of a `host:port` server address (`[v6]:port` too).
pub fn host_of(server: &str) -> &str {
    let h = server.rsplit_once(':').map_or(server, |(h, _)| h);
    h.trim_start_matches('[').trim_end_matches(']')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_split_from_ports() {
        assert_eq!(host_of("1.2.3.4:15151"), "1.2.3.4");
        assert_eq!(host_of("[::1]:15151"), "::1");
        assert_eq!(host_of("example"), "example");
    }

    #[test]
    fn datagrams_go_both_ways_and_strangers_are_dropped() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let link = UdpLink::open("127.0.0.1", port).unwrap();
        assert_eq!(link.server().port(), port);
        assert!(link.send(b"hello"));
        let mut buf = [0u8; 16];
        let (n, from) = server.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
        let stranger = UdpSocket::bind("127.0.0.1:0").unwrap();
        stranger.send_to(b"spam", from).unwrap();
        server.send_to(b"voice", from).unwrap();
        let mut got = Vec::new();
        while got.is_empty() {
            got.extend(link.poll());
            std::thread::yield_now();
        }
        assert_eq!(got, vec![b"voice".to_vec()]);
        assert!(link.poll().is_empty());
    }

    #[test]
    fn bad_addresses_fail() {
        assert!(UdpLink::open("no such host.invalid", 1).is_err());
    }
}
