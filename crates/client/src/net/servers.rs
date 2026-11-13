//! The servers a player can pick: the public list, and the servers the
//! player used recently or kept as favorites, in the settings folder.
//!
//! Each list is a text file with one server a line: its address, then
//! optionally a space and its name. Lines starting with `#` are comments.

use std::path::{Path, PathBuf};
use std::time::Duration;

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
    pub address: String,
    /// The name, or the address if the list gives none.
    pub name: String,
}

/// Reads a list of servers.
pub fn parse(text: &str) -> Vec<ListedServer> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once(char::is_whitespace) {
            Some((a, n)) => ListedServer {
                address: a.to_owned(),
                name: n.trim().to_owned(),
            },
            None => ListedServer {
                address: l.to_owned(),
                name: l.to_owned(),
            },
        })
        .collect()
}

fn write(path: &Path, list: &[ListedServer]) -> std::io::Result<()> {
    let mut text = String::new();
    for s in list {
        text.push_str(&s.address);
        if s.name != s.address {
            text.push(' ');
            text.push_str(&s.name);
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

pub fn is_favorite(paths: &Paths, address: &str) -> bool {
    favorites(paths).iter().any(|s| s.address == address)
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

/// The public list, or an empty one if it cannot be had in time.
pub fn public(url: &str) -> Result<Vec<ListedServer>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(PUBLIC_LIST_TIMEOUT))
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

    #[test]
    fn a_list_has_addresses_and_optional_names() {
        let list = parse("# comment\n\n1.2.3.4:2501 Andrew's server\nhost.example\n");
        assert_eq!(
            list,
            vec![
                ListedServer {
                    address: "1.2.3.4:2501".into(),
                    name: "Andrew's server".into()
                },
                ListedServer {
                    address: "host.example".into(),
                    name: "host.example".into()
                },
            ]
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
            address: a.into(),
            name: a.into(),
        };
        for a in ["a", "b", "a", "c", "d", "e", "f"] {
            add_recent(&paths, s(a)).unwrap();
        }
        let addresses: Vec<String> = recent(&paths).into_iter().map(|s| s.address).collect();
        assert_eq!(addresses, ["f", "e", "d", "c", "a"]);
        toggle_favorite(&paths, s("x")).unwrap();
        assert!(is_favorite(&paths, "x"));
        toggle_favorite(&paths, s("x")).unwrap();
        assert!(!is_favorite(&paths, "x"));
    }
}
