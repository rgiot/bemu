//! `--watch`: the files given on the command line are watched, and taken again when they change
//! (an assembler wrote them anew), for an edit, assemble, see loop.

use std::time::{Duration, Instant, SystemTime};

use amsrust::fdc::FdcDrive;

/// How often the files are looked at.
const POLL: Duration = Duration::from_millis(250);

/// What a watched file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sna,
    Dsk(FdcDrive),
    Cdt
}

/// A file's modification time and size: a change of either is a new file.
type Stamp = Option<(SystemTime, u64)>;

struct Watched {
    path: String,
    kind: Kind,
    /// What was last taken.
    taken: Stamp,
    /// A change seen at the last look, taken once it stays as it is for a look more (the writer
    /// may not be done).
    seen: Stamp
}

fn stamp(path: &str) -> Stamp {
    // a disc in an archive (`games.zip#side b.dsk`): the archive changes
    let path = amsrust::disc_image::split_disc_spec(path).0;
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

pub struct Watcher {
    files: Vec<Watched>,
    next_poll: Instant
}

impl Watcher {
    /// Watch `files`, as they are now.
    pub fn new(files: impl IntoIterator<Item = (String, Kind)>) -> Self {
        let files = files
            .into_iter()
            .map(|(path, kind)| {
                let taken = stamp(&path);
                Watched { path, kind, taken, seen: taken }
            })
            .collect();
        Self { files, next_poll: Instant::now() + POLL }
    }

    /// `path` changed because the emulator wrote it: take it as it is now.
    pub fn retake(&mut self, path: &str) {
        for file in self.files.iter_mut().filter(|file| file.path == path) {
            let now = stamp(&file.path);
            file.taken = now;
            file.seen = now;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|file| file.path.as_str())
    }

    /// The files that changed and are complete, with their paths: cheap, only looks at the files
    /// every [`POLL`].
    pub fn changed(&mut self) -> Vec<(String, Kind)> {
        let now = Instant::now();
        if now < self.next_poll {
            return Vec::new();
        }
        self.next_poll = now + POLL;
        let mut changed = Vec::new();
        for file in &mut self.files {
            let current = stamp(&file.path);
            if current.is_none() || current == file.taken {
                // gone (being rewritten), or as it was
                file.seen = current;
                continue;
            }
            if current == file.seen {
                file.taken = current;
                changed.push((file.path.clone(), file.kind));
            }
            file.seen = current;
        }
        changed
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_change_is_taken_once_the_file_is_stable() {
        let dir = std::env::temp_dir().join(format!("amsrust_watch_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.sna").to_str().unwrap().to_owned();
        std::fs::write(&path, b"one").unwrap();
        let mut watcher = Watcher::new([(path.clone(), Kind::Sna)]);
        let look = |watcher: &mut Watcher| {
            watcher.next_poll = Instant::now();
            watcher.changed()
        };
        assert!(look(&mut watcher).is_empty());
        std::fs::write(&path, b"second").unwrap();
        assert!(look(&mut watcher).is_empty(), "maybe still being written");
        assert_eq!(look(&mut watcher), vec![(path.clone(), Kind::Sna)]);
        assert!(look(&mut watcher).is_empty(), "taken once");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
