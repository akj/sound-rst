//! The servers a player can pick: the public list, and the servers the
//! player used recently or kept as favorites, in the settings folder.
//!
//! Each list is a text file with one server a line: its address, then
//! optionally a space and its name. Lines starting with `#` are comments.
//! An address that starts with `wss://` is a server reached only by an
//! encrypted connection ([`ServerAddress`]); a line whose address is not
//! one is skipped. A name is a [`ServerName`], so it never holds a line
//! break.

use std::path::{Path, PathBuf};
use std::time::Duration;

use soundrts_protocol::ServerName;

use super::conn::ServerAddress;
use crate::paths::Paths;

/// Where the public list is published.
pub const PUBLIC_LIST_URL: &str =
    "https://raw.githubusercontent.com/akj/sound-rst/main/servers.txt";

/// How long to wait for the public list.
const PUBLIC_LIST_TIMEOUT: Duration = Duration::from_secs(3);

/// The most recent servers kept.
const RECENT_LIMIT: usize = 5;

/// A server of a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedServer {
    pub address: ServerAddress,
    pub name: Option<ServerName>,
}

impl ListedServer {
    /// What the player hears: the name, or the address if the list gives
    /// none.
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => name.as_str().to_owned(),
            None => self.address.to_string(),
        }
    }
}

/// Reads a list of servers. A name with characters a name cannot have is
/// cleaned of them.
pub fn parse(text: &str) -> Vec<ListedServer> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (address, name) = match l.split_once(char::is_whitespace) {
                Some((a, n)) => (a, ServerName::clean(n)),
                None => (l, None),
            };
            Some(ListedServer {
                address: ServerAddress::parse(address)?,
                name,
            })
        })
        .collect()
}

fn write(path: &Path, list: &[ListedServer]) -> std::io::Result<()> {
    let mut text = String::new();
    for s in list {
        text.push_str(&s.address.to_string());
        if let Some(name) = &s.name {
            text.push(' ');
            text.push_str(name.as_str());
        }
        text.push('\n');
    }
    std::fs::write(path, text)
}

fn read(path: &Path) -> Vec<ListedServer> {
    std::fs::read_to_string(path)
        .map(|t| parse(&t))
        .unwrap_or_default()
}

pub fn recent_path(paths: &Paths) -> PathBuf {
    paths.user.join("recent_servers.txt")
}

pub fn favorites_path(paths: &Paths) -> PathBuf {
    paths.user.join("favorite_servers.txt")
}

pub fn recent(paths: &Paths) -> Vec<ListedServer> {
    read(&recent_path(paths))
}

pub fn favorites(paths: &Paths) -> Vec<ListedServer> {
    read(&favorites_path(paths))
}

/// Puts a server first in the recent list.
pub fn add_recent(paths: &Paths, server: ListedServer) -> std::io::Result<()> {
    let mut list = recent(paths);
    list.retain(|s| s.address != server.address);
    list.insert(0, server);
    list.truncate(RECENT_LIMIT);
    write(&recent_path(paths), &list)
}

pub fn is_favorite(paths: &Paths, address: &ServerAddress) -> bool {
    favorites(paths).iter().any(|s| s.address == *address)
}

/// Adds a server to the favorites, or removes it if it is one.
pub fn toggle_favorite(paths: &Paths, server: ListedServer) -> std::io::Result<()> {
    let mut list = favorites(paths);
    if list.iter().any(|s| s.address == server.address) {
        list.retain(|s| s.address != server.address);
    } else {
        list.push(server);
    }
    write(&favorites_path(paths), &list)
}

/// The public list, or an empty one if it cannot be had in time. Only
/// over HTTPS: a redirect to anything else is refused.
pub fn public(url: &str) -> Result<Vec<ListedServer>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(PUBLIC_LIST_TIMEOUT))
        .https_only(true)
        .build()
        .into();
    let text = agent
        .get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    Ok(parse(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::conn::Transport;

    fn address(text: &str) -> ServerAddress {
        ServerAddress::parse(text).unwrap()
    }

    #[test]
    fn a_list_has_addresses_and_optional_names() {
        let list = parse(
            "# comment\n\n1.2.3.4:2501 Andrew's server\nhost.example\n\
             wss://games.example.org Encrypted\nhttps://games.example.org Not one\n",
        );
        assert_eq!(
            list,
            vec![
                ListedServer {
                    address: address("1.2.3.4"),
                    name: ServerName::new("Andrew's server").ok()
                },
                ListedServer {
                    address: address("host.example"),
                    name: None
                },
                ListedServer {
                    address: address("wss://games.example.org"),
                    name: ServerName::new("Encrypted").ok()
                },
            ]
        );
        assert_eq!(list[0].label(), "Andrew's server");
        assert_eq!(list[1].label(), "host.example");
        assert_eq!(list[2].address.transport(), Transport::Encrypted);
    }

    #[test]
    fn an_encrypted_server_stays_encrypted_in_the_lists_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            root: dir.path().into(),
            user: dir.path().into(),
        };
        let server = ListedServer {
            address: address("wss://games.example.org:8443"),
            name: None,
        };
        add_recent(&paths, server.clone()).unwrap();
        toggle_favorite(&paths, server.clone()).unwrap();
        assert_eq!(recent(&paths), std::slice::from_ref(&server));
        assert_eq!(favorites(&paths), [server]);
        let text = std::fs::read_to_string(recent_path(&paths)).unwrap();
        assert_eq!(text, "wss://games.example.org:8443\n");
    }

    #[test]
    fn a_server_name_cannot_add_a_line_to_a_list() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            root: dir.path().into(),
            user: dir.path().into(),
        };
        // A server that calls itself this would add the second line, were
        // its name written as it is.
        let name = "bad\n6.6.6.6 trap";
        assert!(ServerName::new(name).is_err());
        let server = ListedServer {
            address: address("1.2.3.4"),
            name: ServerName::clean(name),
        };
        add_recent(&paths, server).unwrap();
        let list = recent(&paths);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].label(), "bad6.6.6.6 trap");
    }

    #[test]
    fn the_public_list_is_not_fetched_over_plain_http() {
        // A plain HTTP server on this machine with a list to give.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let body = "6.6.6.6 trap\n";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let result = public(&format!("http://{addr}/servers.txt"));
        assert!(
            result.as_ref().is_err_and(|e| e.contains("https")),
            "{result:?}"
        );
    }

    #[test]
    fn recent_servers_are_newest_first_without_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            root: dir.path().into(),
            user: dir.path().into(),
        };
        let s = |a: &str| ListedServer {
            address: address(a),
            name: None,
        };
        for a in ["a", "b", "a:2501", "c", "d", "e", "f"] {
            add_recent(&paths, s(a)).unwrap();
        }
        let addresses: Vec<String> = recent(&paths)
            .into_iter()
            .map(|s| s.address.to_string())
            .collect();
        assert_eq!(addresses, ["f", "e", "d", "c", "a"]);
        toggle_favorite(&paths, s("x")).unwrap();
        assert!(is_favorite(&paths, &address("x")));
        assert!(!is_favorite(&paths, &address("wss://x")));
        toggle_favorite(&paths, s("x")).unwrap();
        assert!(!is_favorite(&paths, &address("x")));
    }
}
