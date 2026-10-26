//! The M4 board's WiFi, on the host's network.
//!
//! The board's ESP8266 joins a WiFi network and runs TCP sockets, DNS and HTTP for the CPC. Here
//! nothing is joined: "connected" means the host's network is used (unless the WiFi is turned
//! off), whatever SSID `|NETSET` gives. The commands and the socket table are those of the M4
//! ROM and of Duke's network examples (M4examples: tcp.s, tcpserv.s, lookup.s, httpget.s,
//! telnet.s):
//! - the sockets' state is in the ROM at &FE00 (`sock_info`, found through &FF06): 5 slots of 16
//!   bytes, slot 0 the DNS lookup's, sockets 1-4; in each: the status (0 done, 1 connecting, 2
//!   sending, 3 closed by the peer, 4 accepting, 5 resolving, &F0-&FF an error), the last
//!   command, the bytes waiting (LE), the peer's IP (its first byte last) and port (LE);
//! - a command leaves its socket in the state it starts (connecting, sending...) before it
//!   returns, as the programs poll at once; [`Network::poll`] moves them on as the host's
//!   sockets do.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Duration;

pub const C_HTTPGET: u16 = 0x4320;
pub const C_SETNETWORK: u16 = 0x4321;
pub const C_NETSTAT: u16 = 0x4323;
pub const C_HTTPGETMEM: u16 = 0x4328;
pub const C_COPYBUF: u16 = 0x4329;
pub const C_NETSOCKET: u16 = 0x4331;
pub const C_NETCONNECT: u16 = 0x4332;
pub const C_NETCLOSE: u16 = 0x4333;
pub const C_NETSEND: u16 = 0x4334;
pub const C_NETRECV: u16 = 0x4335;
pub const C_NETHOSTIP: u16 = 0x4336;
pub const C_NETRSSI: u16 = 0x4337;
pub const C_NETBIND: u16 = 0x4338;
pub const C_NETLISTEN: u16 = 0x4339;
pub const C_NETACCEPT: u16 = 0x433a;
pub const C_GETNETWORK: u16 = 0x433b;
pub const C_WIFIPOW: u16 = 0x433c;

const STATUS_DONE: u8 = 0;
const STATUS_CONNECTING: u8 = 1;
const STATUS_SENDING: u8 = 2;
const STATUS_CLOSED: u8 = 3;
const STATUS_ACCEPTING: u8 = 4;
const STATUS_RESOLVING: u8 = 5;
const STATUS_ERROR: u8 = 0xf0;

/// What a receive gives at most (the examples' "max size we can receive in one go").
const RECV_MAX: usize = 2048;
const TIMEOUT: Duration = Duration::from_secs(10);

/// One of the board's sockets.
#[derive(Default)]
struct Socket {
    status: u8,
    last_command: u8,
    stream: Option<TcpStream>,
    listener: Option<TcpListener>,
    connecting: Option<Receiver<std::io::Result<TcpStream>>>,
    bind: Option<SocketAddrV4>,
    peer: Option<SocketAddrV4>,
    received: VecDeque<u8>,
    to_send: VecDeque<u8>,
    peer_closed: bool
}

/// The network side of the board.
pub struct Network {
    /// The WiFi on (the host's network used) or off.
    pub enabled: bool,
    /// Slot 0 is the DNS lookup's; 1-4 the sockets.
    sockets: [Option<Socket>; 5],
    dns_status: u8,
    dns_ip: Ipv4Addr,
    resolving: Option<Receiver<Option<Ipv4Addr>>>,
    /// What HTTPGETMEM fetched, for COPYBUF.
    downloaded: Vec<u8>,
    /// `|NETSET`'s settings (m4/config.txt), `start=1` ones kept for the next call.
    settings: Vec<(String, String)>,
    /// Host names answered with another address (`BEMU_M4_HOSTS=name=ip[:port],...`, or
    /// [`Network::map_host`]): a test's local server in place of a site.
    hosts: Vec<(String, String)>
}

impl Network {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            sockets: Default::default(),
            dns_status: STATUS_DONE,
            dns_ip: Ipv4Addr::UNSPECIFIED,
            resolving: None,
            downloaded: Vec::new(),
            settings: Vec::new(),
            hosts: std::env::var("BEMU_M4_HOSTS").map(|hosts| parse_hosts(&hosts)).unwrap_or_default()
        }
    }

    /// Answer `name` (DNS, HTTP) with `target`: an IP, with a port for HTTP (`127.0.0.1:8080`).
    pub fn map_host(&mut self, name: &str, target: &str) {
        self.hosts.push((name.to_ascii_lowercase(), target.to_owned()));
    }

    /// The board's settings from `m4/config.txt` on the SD card.
    pub fn load_settings(&mut self, sd: &Path) {
        if let Ok(text) = std::fs::read_to_string(sd.join("m4").join("config.txt")) {
            self.settings = parse_settings(&text.replace('\n', ","));
        }
    }

    fn mapped(&self, name: &str) -> Option<&str> {
        self.hosts.iter().rev().find(|(known, _)| known.eq_ignore_ascii_case(name)).map(|(_, target)| target.as_str())
    }

    /// The URL with its host mapped (plain HTTP then, a local server has no TLS).
    fn rewrite(&self, url: &str) -> String {
        let url = url.trim();
        let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")).unwrap_or(url);
        let end = rest.find(['/', ':']).unwrap_or(rest.len());
        match self.mapped(&rest[..end]) {
            Some(target) => {
                let path = rest[end..].trim_start_matches(|c: char| c == ':' || c.is_ascii_digit());
                format!("http://{target}{path}")
            },
            None => url.to_owned()
        }
    }

    fn setting(&self, key: &str) -> Option<&str> {
        self.settings.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    /// The time zone set (`tz`, hours).
    pub fn time_zone(&self) -> i64 {
        self.setting("tz").and_then(|tz| tz.trim().trim_start_matches('+').parse().ok()).unwrap_or(0)
    }

    pub fn reset(&mut self) {
        self.sockets = Default::default();
        self.dns_status = STATUS_DONE;
        self.resolving = None;
        self.downloaded.clear();
    }

    /// A network command: its payload, or `None` when not one of them. `directory` is where
    /// HTTPGET saves.
    pub fn command(&mut self, command: u16, params: &[u8], directory: &Path, sd: &Path) -> Option<Vec<u8>> {
        let byte = |index: usize| params.get(index).copied().unwrap_or(0);
        let word = |index: usize| u16::from_le_bytes([byte(index), byte(index + 1)]);
        let socket_number = usize::from(byte(0));
        let payload = match command {
            C_WIFIPOW => {
                self.enabled = byte(0) != 0;
                if !self.enabled {
                    self.reset();
                }
                Vec::new()
            },
            C_NETSTAT => match (self.enabled, host_ip()) {
                (true, Some(ip)) => {
                    let mut payload = nul_terminated(&format!("IP: {ip}"));
                    payload.push(5);
                    payload
                },
                (true, None) => {
                    let mut payload = nul_terminated("No network on the host");
                    payload.push(0);
                    payload
                },
                (false, _) => {
                    let mut payload = nul_terminated("WiFi is off");
                    payload.push(0);
                    payload
                }
            },
            // a signal of -50 dBm; 31: none
            C_NETRSSI => vec![if self.enabled { 0xce } else { 31 }],
            C_GETNETWORK => self.network_info(),
            C_SETNETWORK => {
                self.set_network(&string(params), sd);
                Vec::new()
            },
            C_NETSOCKET => {
                let free = (1..5).find(|&number| self.sockets[number].is_none());
                match (self.enabled, free) {
                    (true, Some(number)) => {
                        self.sockets[number] = Some(Socket::default());
                        vec![number as u8]
                    },
                    _ => vec![0xff]
                }
            },
            C_NETCLOSE => {
                if (1..5).contains(&socket_number) {
                    self.sockets[socket_number] = None;
                }
                Vec::new()
            },
            C_NETCONNECT => {
                let address = SocketAddrV4::new(reversed_ip(&params[1.min(params.len())..]), word(5));
                match self.socket(socket_number) {
                    Some(socket) => {
                        let (sender, receiver) = channel();
                        std::thread::spawn(move || {
                            let _ = sender.send(TcpStream::connect_timeout(&SocketAddr::V4(address), TIMEOUT));
                        });
                        socket.connecting = Some(receiver);
                        socket.peer = Some(address);
                        socket.status = STATUS_CONNECTING;
                        socket.last_command = 3;
                        vec![0]
                    },
                    None => vec![0xff]
                }
            },
            C_NETSEND => {
                let size = usize::from(word(1));
                let data = params.get(3..).unwrap_or(&[]);
                let data = &data[..size.min(data.len())];
                match self.socket(socket_number) {
                    Some(socket) => {
                        socket.to_send.extend(data);
                        socket.status = STATUS_SENDING;
                        socket.last_command = 1;
                        vec![0]
                    },
                    None => vec![0xff]
                }
            },
            C_NETRECV => {
                let wanted = usize::from(word(1)).min(RECV_MAX);
                match self.socket(socket_number) {
                    Some(socket) => {
                        let count = wanted.min(socket.received.len());
                        let mut payload = vec![0];
                        payload.extend_from_slice(&(count as u16).to_le_bytes());
                        payload.extend(socket.received.drain(..count));
                        socket.last_command = 5;
                        if socket.peer_closed && socket.received.is_empty() {
                            socket.status = STATUS_CLOSED;
                        }
                        payload
                    },
                    None => vec![0xff, 0, 0]
                }
            },
            C_NETHOSTIP => {
                if !self.enabled {
                    return Some(vec![0xff]);
                }
                let name = string(params);
                let name = match self.mapped(&name) {
                    Some(target) => target.split(':').next().unwrap_or(target).to_owned(),
                    None => name
                };
                let (sender, receiver) = channel();
                std::thread::spawn(move || {
                    let ip = (name.as_str(), 0).to_socket_addrs().ok().and_then(|mut addresses| {
                        addresses.find_map(|address| match address {
                            SocketAddr::V4(v4) => Some(*v4.ip()),
                            SocketAddr::V6(_) => None
                        })
                    });
                    let _ = sender.send(ip);
                });
                self.resolving = Some(receiver);
                self.dns_status = STATUS_RESOLVING;
                vec![1]
            },
            C_NETBIND => {
                let address = SocketAddrV4::new(reversed_ip(&params[1.min(params.len())..]), word(5));
                match self.socket(socket_number) {
                    Some(socket) => {
                        socket.bind = Some(address);
                        vec![0]
                    },
                    None => vec![0xff]
                }
            },
            C_NETLISTEN => match self.socket(socket_number) {
                Some(socket) => {
                    let address = socket.bind.unwrap_or(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
                    match TcpListener::bind(address).and_then(|listener| {
                        listener.set_nonblocking(true)?;
                        Ok(listener)
                    }) {
                        Ok(listener) => {
                            socket.listener = Some(listener);
                            vec![0]
                        },
                        Err(_) => vec![0xff]
                    }
                },
                None => vec![0xff]
            },
            C_NETACCEPT => match self.socket(socket_number) {
                Some(socket) if socket.listener.is_some() => {
                    socket.status = STATUS_ACCEPTING;
                    socket.last_command = 4;
                    vec![0]
                },
                _ => vec![0xff]
            },
            C_HTTPGET => {
                let message = if self.enabled {
                    match http_get(&self.rewrite(&string(params))) {
                        Ok((name, body)) => {
                            let path = directory.join(&name);
                            match std::fs::write(&path, &body) {
                                Ok(()) => format!("Downloaded {name} ({} bytes)", body.len()),
                                Err(e) => format!("{name}: {e}")
                            }
                        },
                        Err(e) => e
                    }
                }
                else {
                    "WiFi is off".to_owned()
                };
                nul_terminated(&message)
            },
            C_HTTPGETMEM => {
                // the size, the URL (", offset=N" after it reads from N)
                let size = usize::from(word(0));
                let url = string(params.get(2..).unwrap_or(&[]));
                let (url, offset) = match url.split_once(", offset=") {
                    Some((url, offset)) => (url.to_owned(), parse_number(offset).unwrap_or(0)),
                    None => (url, 0)
                };
                self.downloaded = if self.enabled {
                    http_get(&self.rewrite(&url)).map(|(_, body)| body.into_iter().skip(offset).take(size).collect()).unwrap_or_default()
                }
                else {
                    Vec::new()
                };
                (self.downloaded.len() as u16).to_le_bytes().to_vec()
            },
            C_COPYBUF => {
                let (offset, count) = (usize::from(word(0)), usize::from(word(2)).min(0x200));
                let mut data: Vec<u8> = self.downloaded.iter().skip(offset).take(count).copied().collect();
                data.resize(count, 0);
                data
            },
            _ => return None
        };
        self.poll();
        Some(payload)
    }

    fn socket(&mut self, number: usize) -> Option<&mut Socket> {
        if (1..5).contains(&number) { self.sockets[number].as_mut() } else { None }
    }

    /// The host's sockets moved on: connections made, data sent and received, clients
    /// accepted, lookups done.
    pub fn poll(&mut self) {
        if let Some(receiver) = &self.resolving {
            match receiver.try_recv() {
                Ok(ip) => {
                    self.resolving = None;
                    match ip {
                        Some(ip) => {
                            self.dns_ip = ip;
                            self.dns_status = STATUS_DONE;
                        },
                        None => self.dns_status = STATUS_ERROR
                    }
                },
                Err(TryRecvError::Empty) => {},
                Err(TryRecvError::Disconnected) => {
                    self.resolving = None;
                    self.dns_status = STATUS_ERROR;
                }
            }
        }
        for socket in self.sockets.iter_mut().flatten() {
            if let Some(receiver) = &socket.connecting {
                match receiver.try_recv() {
                    Ok(Ok(stream)) => {
                        let _ = stream.set_nonblocking(true);
                        let _ = stream.set_nodelay(true);
                        socket.stream = Some(stream);
                        socket.connecting = None;
                        socket.status = STATUS_DONE;
                    },
                    Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                        socket.connecting = None;
                        socket.status = STATUS_ERROR;
                    },
                    Err(TryRecvError::Empty) => {}
                }
            }
            if socket.status == STATUS_ACCEPTING {
                if let Some(listener) = &socket.listener {
                    if let Ok((stream, SocketAddr::V4(peer))) = listener.accept() {
                        let _ = stream.set_nonblocking(true);
                        let _ = stream.set_nodelay(true);
                        socket.stream = Some(stream);
                        socket.peer = Some(peer);
                        socket.status = STATUS_DONE;
                    }
                }
            }
            let Some(stream) = &mut socket.stream else { continue };
            while !socket.to_send.is_empty() {
                let (front, _) = socket.to_send.as_slices();
                match stream.write(front) {
                    Ok(0) => break,
                    Ok(written) => {
                        socket.to_send.drain(..written);
                    },
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        socket.to_send.clear();
                        socket.status = STATUS_ERROR;
                    }
                }
            }
            if socket.status == STATUS_SENDING && socket.to_send.is_empty() {
                socket.status = STATUS_DONE;
            }
            let mut buffer = [0u8; 4096];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => {
                        socket.peer_closed = true;
                        break;
                    },
                    Ok(count) => socket.received.extend(&buffer[..count]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        socket.peer_closed = true;
                        break;
                    }
                }
            }
            if socket.peer_closed && socket.received.is_empty() && socket.status == STATUS_DONE {
                socket.status = STATUS_CLOSED;
            }
        }
    }

    /// The socket table (`sock_info`, 80 bytes), as the ROM keeps it at &FE00.
    pub fn sock_info(&self) -> [u8; 80] {
        let mut table = [0u8; 80];
        table[0] = self.dns_status;
        table[1] = 2;
        table[4..8].copy_from_slice(&reversed_bytes(self.dns_ip));
        for (number, socket) in self.sockets.iter().enumerate().skip(1) {
            let Some(socket) = socket else { continue };
            let slot = &mut table[number * 16..number * 16 + 16];
            slot[0] = socket.status;
            slot[1] = socket.last_command;
            slot[2..4].copy_from_slice(&(socket.received.len().min(0xffff) as u16).to_le_bytes());
            if let Some(peer) = socket.peer {
                slot[4..8].copy_from_slice(&reversed_bytes(*peer.ip()));
                slot[8..10].copy_from_slice(&peer.port().to_le_bytes());
            }
        }
        table
    }

    /// C_GETNETWORK: the IP at 112, the netmask, the gateway, the DNS servers, the MAC at 190
    /// (network order).
    fn network_info(&self) -> Vec<u8> {
        let mut payload = vec![0u8; 196];
        let ip_setting = |key: &str| self.setting(key).and_then(|value| value.trim().parse::<Ipv4Addr>().ok());
        let fields = [
            (112, if self.enabled { host_ip() } else { None }),
            (116, ip_setting("nm").or(Some(Ipv4Addr::new(255, 255, 255, 0)))),
            (120, ip_setting("gw")),
            (124, ip_setting("dns1")),
            (128, ip_setting("dns2"))
        ];
        for (offset, ip) in fields {
            if let Some(ip) = ip {
                payload[offset..offset + 4].copy_from_slice(&ip.octets());
            }
        }
        payload[190..196].copy_from_slice(&[0x02, 0xbe, 0x40, 0x00, 0x00, 0x04]);
        payload
    }

    /// `|NETSET,"name=…, ssid=…, …"`: the settings kept in m4/config.txt (nothing is joined).
    fn set_network(&mut self, text: &str, sd: &Path) {
        for (key, value) in parse_settings(text) {
            self.settings.retain(|(k, _)| !k.eq_ignore_ascii_case(&key));
            self.settings.push((key, value));
        }
        if self.setting("start") == Some("1") {
            return;
        }
        let file: String = self
            .settings
            .iter()
            .filter(|(key, _)| !key.eq_ignore_ascii_case("start"))
            .map(|(key, value)| format!("{key}={value}\n"))
            .collect();
        let directory = sd.join("m4");
        let _ = std::fs::create_dir_all(&directory);
        let _ = std::fs::write(directory.join("config.txt"), file);
    }
}

/// "key=value" pairs, separated by commas.
fn parse_settings(text: &str) -> Vec<(String, String)> {
    text.split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .filter(|(key, _)| !key.is_empty())
        .collect()
}

/// An IP as the board's socket commands give it: its first byte last.
fn reversed_ip(bytes: &[u8]) -> Ipv4Addr {
    let byte = |index: usize| bytes.get(index).copied().unwrap_or(0);
    Ipv4Addr::new(byte(3), byte(2), byte(1), byte(0))
}

fn reversed_bytes(ip: Ipv4Addr) -> [u8; 4] {
    let [a, b, c, d] = ip.octets();
    [d, c, b, a]
}

/// The host's own address on its network (where it would send from; nothing is sent).
fn host_ip() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()? {
        SocketAddr::V4(address) if !address.ip().is_unspecified() => Some(*address.ip()),
        _ => Some(Ipv4Addr::LOCALHOST)
    }
}

fn string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

fn nul_terminated(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

fn parse_number(text: &str) -> Option<usize> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("&")).or_else(|| text.strip_prefix("#")) {
        Some(hex) => usize::from_str_radix(hex, 16).ok(),
        None => text.parse().ok()
    }
}

/// An HTTP GET (`host[:port]/path`, `http://` optional): the file's name and its body. Plain
/// HTTP as the board; a redirection to HTTPS is followed too (the board's ESP can't), 4 at most.
pub fn http_get(url: &str) -> Result<(String, Vec<u8>), String> {
    let url = url.trim();
    // the board's URLs have no scheme: plain HTTP (a site redirecting to HTTPS is followed)
    let mut url = if url.contains("://") { url.to_owned() } else { format!("http://{url}") };
    for _ in 0..5 {
        let response = agent_for(&url).get(&url).call().map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("{url}: HTTP {code}"),
            ureq::Error::Transport(transport) => format!("{url}: {transport}")
        })?;
        if (300..400).contains(&response.status()) {
            let location = response.header("location").ok_or_else(|| format!("{url}: redirected nowhere"))?;
            url = if location.starts_with('/') {
                // the same scheme and host
                let origin_end = url.find("://").map_or(0, |at| at + 3);
                let host_end = url[origin_end..].find('/').map_or(url.len(), |at| origin_end + at);
                format!("{}{location}", &url[..host_end])
            }
            else {
                location.to_owned()
            };
            continue;
        }
        // the name the server gives (cpcrulez.fr's `?d=` downloads), else the path's last part
        let path = response.get_url().split('?').next().unwrap_or_default().to_owned();
        let attachment = response.header("content-disposition").and_then(attachment_name);
        let name = attachment.unwrap_or_else(|| {
            let last = path.rsplit('/').next().unwrap_or_default();
            if last.is_empty() || last.contains(':') { "index.html".to_owned() } else { last.to_owned() }
        });
        let mut body = Vec::new();
        response.into_reader().take(16 << 20).read_to_end(&mut body).map_err(|e| format!("{url}: {e}"))?;
        return Ok((name, body));
    }
    Err(format!("{url}: too many redirections"))
}

/// `attachment; filename="NAME"`'s NAME, without any folder (it lands in the current one).
fn attachment_name(header: &str) -> Option<String> {
    let (_, value) = header.split_once("filename=")?;
    let value = value.split(';').next()?.trim().trim_matches('"');
    let name = value.rsplit(['/', '\\']).next()?.trim();
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_owned())
}

/// A direct connection, or the host's HTTPS proxy (`HTTPS_PROXY`, unless `NO_PROXY` names the
/// host) for an `https://` URL.
fn agent_for(url: &str) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new().timeout(TIMEOUT).user_agent("M4 (bemu)").redirects(0).try_proxy_from_env(false);
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':', '?']).next().unwrap_or_default();
    let proxy = url
        .starts_with("https://")
        .then(|| std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("https_proxy")).ok())
        .flatten()
        .filter(|_| !no_proxy(host))
        .and_then(|proxy| ureq::Proxy::new(proxy).ok());
    match proxy {
        Some(proxy) => builder.proxy(proxy),
        None => builder
    }
    .build()
}

fn no_proxy(host: &str) -> bool {
    let list = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
    list.split(',').map(|entry| entry.trim().trim_start_matches('*').trim_start_matches('.')).any(|entry| {
        !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
    })
}

/// `name=target,...`
fn parse_hosts(text: &str) -> Vec<(String, String)> {
    text.split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, target)| (name.trim().to_ascii_lowercase(), target.trim().to_owned()))
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn network() -> Network {
        Network::new(true)
    }

    /// Poll until `done` (2 s at most).
    fn wait(network: &mut Network, done: impl Fn(&[u8; 80]) -> bool) -> [u8; 80] {
        for _ in 0..200 {
            network.poll();
            let table = network.sock_info();
            if done(&table) {
                return table;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        network.sock_info()
    }

    fn sd() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    /// tcp.s's sequence against an echo server on the host.
    #[test]
    fn a_tcp_client_talks_to_the_host() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = server.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            (&stream).write_all(line.to_uppercase().as_bytes()).unwrap();
        });
        let mut net = network();
        let socket = net.command(C_NETSOCKET, &[0, 0, 6], &sd(), &sd()).unwrap()[0];
        assert_eq!(socket, 1);
        let mut connect = vec![socket, 1, 0, 0, 127];
        connect.extend_from_slice(&port.to_le_bytes());
        assert_eq!(net.command(C_NETCONNECT, &connect, &sd(), &sd()).unwrap(), vec![0]);
        let slot = 16 * usize::from(socket);
        let table = wait(&mut net, |t| t[slot] != STATUS_CONNECTING);
        assert_eq!(table[slot], STATUS_DONE, "connected");
        let mut send = vec![socket, 6, 0];
        send.extend_from_slice(b"hello\n");
        net.command(C_NETSEND, &send, &sd(), &sd());
        let table = wait(&mut net, |t| t[slot + 2] >= 6);
        assert_eq!(table[slot + 2], 6, "6 bytes waiting");
        let received = net.command(C_NETRECV, &[socket, 255, 0], &sd(), &sd()).unwrap();
        assert_eq!(&received[..3], &[0, 6, 0]);
        assert_eq!(&received[3..], b"HELLO\n");
        let table = wait(&mut net, |t| t[slot] == STATUS_CLOSED);
        assert_eq!(table[slot], STATUS_CLOSED, "the server closed");
        net.command(C_NETCLOSE, &[socket], &sd(), &sd());
        assert_eq!(net.sock_info()[slot], 0);
    }

    /// tcpserv.s's sequence: a host client connects to the CPC.
    #[test]
    fn a_tcp_server_accepts_a_host_client() {
        let mut net = network();
        let socket = net.command(C_NETSOCKET, &[0, 0, 6], &sd(), &sd()).unwrap()[0];
        // a free port of the host
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut bind = vec![socket, 0, 0, 0, 0];
        bind.extend_from_slice(&port.to_le_bytes());
        assert_eq!(net.command(C_NETBIND, &bind, &sd(), &sd()).unwrap(), vec![0]);
        assert_eq!(net.command(C_NETLISTEN, &[socket], &sd(), &sd()).unwrap(), vec![0]);
        assert_eq!(net.command(C_NETACCEPT, &[socket], &sd(), &sd()).unwrap(), vec![0]);
        let slot = 16 * usize::from(socket);
        assert_eq!(net.sock_info()[slot], STATUS_ACCEPTING);
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let table = wait(&mut net, |t| t[slot] != STATUS_ACCEPTING);
        assert_eq!(table[slot], STATUS_DONE);
        assert_eq!(&table[slot + 4..slot + 8], &[1, 0, 0, 127], "the peer, reversed");
        client.write_all(b"ping").unwrap();
        wait(&mut net, |t| t[slot + 2] == 4);
        let received = net.command(C_NETRECV, &[socket, 255, 0], &sd(), &sd()).unwrap();
        assert_eq!(&received[3..], b"ping");
        let mut send = vec![socket, 4, 0];
        send.extend_from_slice(b"pong");
        net.command(C_NETSEND, &send, &sd(), &sd());
        wait(&mut net, |t| t[slot] != STATUS_SENDING);
        let mut answer = [0u8; 4];
        client.read_exact(&mut answer).unwrap();
        assert_eq!(&answer, b"pong");
    }

    #[test]
    fn dns_lookups_land_in_slot_0() {
        let mut net = network();
        let mut name = b"localhost".to_vec();
        name.push(0);
        assert_eq!(net.command(C_NETHOSTIP, &name, &sd(), &sd()).unwrap(), vec![1]);
        let table = wait(&mut net, |t| t[0] != STATUS_RESOLVING);
        assert_eq!(table[0], STATUS_DONE);
        assert_eq!(&table[4..8], &[1, 0, 0, 127]);
    }

    #[test]
    fn http_downloads_to_a_file_and_to_memory() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = server.accept().unwrap();
                let mut request = [0u8; 512];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 10\r\n\r\n0123456789");
            }
        });
        let directory = std::env::temp_dir().join(format!("bemu-m4-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut net = network();
        let url = format!("127.0.0.1:{port}/files/data.bin\0");
        let message = net.command(C_HTTPGET, url.as_bytes(), &directory, &directory).unwrap();
        assert!(String::from_utf8_lossy(&message).starts_with("Downloaded data.bin (10 bytes)"));
        assert_eq!(std::fs::read(directory.join("data.bin")).unwrap(), b"0123456789");
        // HTTPMEM: 4 bytes from offset 3, then COPYBUF
        let mut params = 4u16.to_le_bytes().to_vec();
        params.extend_from_slice(format!("127.0.0.1:{port}/x, offset=3\0").as_bytes());
        assert_eq!(net.command(C_HTTPGETMEM, &params, &directory, &directory).unwrap(), vec![4, 0]);
        assert_eq!(net.command(C_COPYBUF, &[0, 0, 4, 0], &directory, &directory).unwrap(), b"3456");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_mapped_host_and_the_name_a_server_gives() {
        let mut net = network();
        net.map_host("cpcrulez.fr", "127.0.0.1:8080");
        assert_eq!(net.rewrite("cpcrulez.fr/m4/?d=00192778"), "http://127.0.0.1:8080/m4/?d=00192778");
        assert_eq!(net.rewrite("http://cpcrulez.fr:80/x"), "http://127.0.0.1:8080/x");
        assert_eq!(net.rewrite("example.org/x"), "example.org/x", "other hosts as they are");
        assert_eq!(attachment_name("attachment; filename=\"Batman_2.dsk\"").as_deref(), Some("Batman_2.dsk"));
        assert_eq!(attachment_name("attachment; filename=\"../../etc/passwd\"").as_deref(), Some("passwd"));
        assert_eq!(attachment_name("inline"), None);
    }

    #[test]
    fn the_wifi_off_has_no_network() {
        let mut net = Network::new(false);
        assert_eq!(net.command(C_NETSOCKET, &[0, 0, 6], &sd(), &sd()).unwrap(), vec![0xff]);
        assert_eq!(net.command(C_NETRSSI, &[], &sd(), &sd()).unwrap(), vec![31]);
        let status = net.command(C_NETSTAT, &[], &sd(), &sd()).unwrap();
        assert_eq!(*status.last().unwrap(), 0, "not connected");
        net.command(C_WIFIPOW, &[1], &sd(), &sd());
        assert_eq!(*net.command(C_NETSTAT, &[], &sd(), &sd()).unwrap().last().unwrap(), 5, "connected");
    }

    #[test]
    fn netset_writes_the_config() {
        let sd = std::env::temp_dir().join(format!("bemu-m4-netset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sd);
        std::fs::create_dir_all(&sd).unwrap();
        let mut net = network();
        net.command(C_SETNETWORK, b"name=CPC6128, ssid=NETGEAR, pw=12345678, dhcp=1, tz=2\0", &sd, &sd);
        let config = std::fs::read_to_string(sd.join("m4/config.txt")).unwrap();
        assert!(config.contains("ssid=NETGEAR\n") && config.contains("tz=2\n"), "{config}");
        let mut other = network();
        other.load_settings(&sd);
        assert_eq!(other.time_zone(), 2);
        let _ = std::fs::remove_dir_all(&sd);
    }
}
