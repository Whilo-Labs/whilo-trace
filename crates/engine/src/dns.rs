//! Reverse DNS for hop addresses, looked up in the background and cached.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct DnsCache {
    names: Arc<Mutex<HashMap<IpAddr, Option<String>>>>,
    queue: Sender<IpAddr>,
}

impl DnsCache {
    pub fn new() -> Self {
        let names: Arc<Mutex<HashMap<IpAddr, Option<String>>>> = Arc::default();
        let (queue, rx) = channel::<IpAddr>();
        let shared = names.clone();
        std::thread::Builder::new()
            .name("reverse dns".into())
            .spawn(move || {
                for ip in rx {
                    let name = dns_lookup::lookup_addr(&ip)
                        .ok()
                        .filter(|n| *n != ip.to_string());
                    shared.lock().unwrap().insert(ip, name);
                }
            })
            .expect("spawn reverse dns thread");
        Self { names, queue }
    }

    /// The cached name, or `None` while the lookup is pending or when there is no PTR record.
    pub fn name(&self, ip: IpAddr) -> Option<String> {
        let mut names = self.names.lock().unwrap();
        match names.get(&ip) {
            Some(name) => name.clone(),
            None => {
                names.insert(ip, None);
                let _ = self.queue.send(ip);
                None
            }
        }
    }
}

impl Default for DnsCache {
    fn default() -> Self {
        Self::new()
    }
}
