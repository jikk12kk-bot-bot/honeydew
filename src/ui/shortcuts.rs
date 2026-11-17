//! Folder shortcuts (Honeydew addition): rows at a browser's root, after `My Files`, that open a folder of the user
//! folder at once, its path from the user folder on a line under the name (none for a folder right in the user
//! folder, whose name says it all).
//!
//! Each browser has its own default shortcuts, the folders its files are saved in (a module's `My Presets/<module>`,
//! the sample pickers' `My Samples`, SONGS' `My Songs` …), and lists the ones the user added with the ≡ menu's
//! `Add shortcut` in a browser of its scope (every SuperSaw's picker, SONGS). `Remove shortcut` removes one from the
//! browsers of its scope, a default one until it is added again. A folder that is not there (deleted, moved outside
//! the browser) lists no row.
//!
//! SETTINGS keeps them (`folderShortcuts`, `hiddenShortcuts`): the app shares them through egui's memory every frame
//! ([`share`]) and keeps what a browser changed ([`take_edit`]), so each browser needs no settings of its own.

use std::path::{Path, PathBuf};

use eframe::egui::{Context, Id};

/// The shortcuts the user added and the default ones they removed, each folder by its path in the user folder
/// ([`key`]) with the scope of its browsers ([`scoped`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shortcuts {
    /// The added folders, oldest first.
    pub added: Vec<String>,
    /// The removed default shortcuts.
    pub hidden: Vec<String>,
}

impl Shortcuts {
    /// The shortcuts of the browsers of `scope`, whose defaults are `defaults`: the defaults not removed, then the
    /// ones added in them, each once.
    pub fn of(&self, scope: &str, defaults: &[String]) -> Vec<String> {
        let shown = (defaults.iter()).filter(|d| !self.hidden.contains(&scoped(scope, d)));
        let mut keys: Vec<String> = shown.cloned().collect();
        let added = self.added.iter().filter_map(|a| in_scope(scope, a));
        for key in added {
            if !keys.iter().any(|k| k == key) {
                keys.push(key.to_owned());
            }
        }
        keys
    }

    /// `Add shortcut` on the folder `key` in a browser of `scope`: a removed default comes back, any other folder is
    /// added to the browsers of `scope`.
    pub fn add(&mut self, scope: &str, defaults: &[String], key: &str) {
        let scoped = scoped(scope, key);
        if defaults.iter().any(|d| d == key) {
            self.hidden.retain(|h| *h != scoped);
        } else if !self.added.contains(&scoped) {
            self.added.push(scoped);
        }
    }

    /// `Remove shortcut` on the folder `key` in a browser of `scope`: it goes from the browsers of `scope`.
    pub fn remove(&mut self, scope: &str, defaults: &[String], key: &str) {
        let scoped = scoped(scope, key);
        self.added.retain(|a| *a != scoped);
        if defaults.iter().any(|d| d == key) && !self.hidden.contains(&scoped) {
            self.hidden.push(scoped);
        }
    }

    /// The folder `from` was moved or renamed to `to`: the added shortcuts to it, or to a folder inside it, follow,
    /// in every scope. Whether any did.
    pub fn moved(&mut self, from: &str, to: &str) -> bool {
        let follow = |key: &str| -> Option<String> {
            if key == from {
                return Some(to.to_owned());
            }
            let rest = key.strip_prefix(from)?.strip_prefix('/')?;
            Some(format!("{to}/{rest}"))
        };
        let mut changed = false;
        for added in &mut self.added {
            let Some((scope, key)) = added.split_once('|') else {
                continue;
            };
            if let Some(new) = follow(key) {
                *added = scoped(scope, &new);
                changed = true;
            }
        }
        changed
    }
}

/// A shortcut as [`Shortcuts`] keeps it: its browsers' scope, then the folder ([`key`]).
pub fn scoped(scope: &str, key: &str) -> String {
    format!("{scope}|{key}")
}

/// The folder of the kept shortcut `scoped` when it is one of `scope`'s browsers'.
fn in_scope<'a>(scope: &str, scoped: &'a str) -> Option<&'a str> {
    scoped.strip_prefix(scope)?.strip_prefix('|')
}

/// The folder `path`, in the user folder `user`, as a shortcut keeps it: its folders from the user folder, joined by
/// `/`. `None` for the user folder itself and anything outside it.
pub fn key(user: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(user).ok()?;
    let parts: Vec<String> = (rest.components())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The folder of the shortcut `key` in the user folder `user`.
pub fn path(user: &Path, key: &str) -> PathBuf {
    key.split('/').fold(user.to_owned(), |path, part| path.join(part))
}

/// The shortcut row's name: its folder's.
pub fn name(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// The line under the shortcut row: its path from the user folder (`My Presets/SuperSaw`); none for a folder right in
/// the user folder (`My Presets`), whose name is its path.
pub fn sub(key: &str) -> Option<String> {
    key.contains('/').then(|| key.to_owned())
}

fn shared() -> Id {
    Id::new("folder shortcuts")
}

fn edited() -> Id {
    Id::new("folder shortcuts edited")
}

/// The app's shortcuts, for this frame's browsers: call it before they show.
pub fn share(ctx: &Context, shortcuts: Shortcuts) {
    ctx.data_mut(|d| d.insert_temp(shared(), shortcuts));
}

/// The shortcuts of every scope, for a browser to list its own ([`Shortcuts::of`]).
pub fn get(ctx: &Context) -> Shortcuts {
    ctx.data(|d| d.get_temp::<Shortcuts>(shared())).unwrap_or_default()
}

/// A browser changed them: the browsers of its scope list them at once, and the app keeps them ([`take_edit`]).
pub fn set(ctx: &Context, shortcuts: Shortcuts) {
    ctx.data_mut(|d| {
        d.insert_temp(shared(), shortcuts.clone());
        d.insert_temp(edited(), shortcuts);
    });
}

/// What a browser changed since the last call, for SETTINGS to keep.
pub fn take_edit(ctx: &Context) -> Option<Shortcuts> {
    ctx.data_mut(|d| d.remove_temp::<Shortcuts>(edited()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Vec<String> {
        vec!["My Presets/SuperSaw".to_owned()]
    }

    #[test]
    fn a_browser_lists_its_defaults_then_the_ones_added_in_its_scope_each_once() {
        let mut s = Shortcuts::default();
        assert_eq!(s.of("SuperSaw", &defaults()), ["My Presets/SuperSaw"]);
        s.add("SuperSaw", &defaults(), "My Samples/Drums");
        s.add("SuperSaw", &defaults(), "My Samples/Drums");
        assert_eq!(s.added, ["SuperSaw|My Samples/Drums"]);
        assert_eq!(
            s.of("SuperSaw", &defaults()),
            ["My Presets/SuperSaw", "My Samples/Drums"]
        );
        // Only the browsers of its scope list an added one; a scope named as its start is another.
        assert!(s.of("Reverb", &[]).is_empty());
        assert!(s.of("Super", &[]).is_empty());
        s.add("Reverb", &[], "My Samples/Drums");
        assert_eq!(s.of("Reverb", &[]), ["My Samples/Drums"]);
    }

    #[test]
    fn a_removed_default_stays_gone_from_its_scope_until_added_again() {
        let mut s = Shortcuts::default();
        s.remove("SuperSaw", &defaults(), "My Presets/SuperSaw");
        assert!(s.of("SuperSaw", &defaults()).is_empty());
        // Another scope with the same default keeps it.
        assert_eq!(s.of("Other", &defaults()), ["My Presets/SuperSaw"]);
        s.add("SuperSaw", &defaults(), "My Presets/SuperSaw");
        assert_eq!(s, Shortcuts::default());
        // An added one goes from the browsers of its scope.
        s.add("Reverb", &[], "Kicks");
        s.add("Delay", &[], "Kicks");
        s.remove("Reverb", &[], "Kicks");
        assert!(s.of("Reverb", &[]).is_empty());
        assert_eq!(s.of("Delay", &[]), ["Kicks"]);
    }

    #[test]
    fn shortcuts_follow_a_moved_folder() {
        let mut s = Shortcuts {
            added: vec![
                "Songs|My Samples/Drums".into(),
                "Reverb|My Samples/Drums/Kicks".into(),
                "Songs|My Samples/Drumsets".into(),
            ],
            hidden: Vec::new(),
        };
        assert!(s.moved("My Samples/Drums", "My Samples/Beats"));
        assert_eq!(
            s.added,
            [
                "Songs|My Samples/Beats",
                "Reverb|My Samples/Beats/Kicks",
                "Songs|My Samples/Drumsets"
            ]
        );
        assert!(!s.moved("Elsewhere", "Gone"));
    }

    #[test]
    fn a_key_is_the_path_from_the_user_folder() {
        let user = Path::new("/home/me/Honeydew");
        let kicks = user.join("My Samples").join("Kicks");
        assert_eq!(key(user, &kicks).as_deref(), Some("My Samples/Kicks"));
        assert_eq!(path(user, "My Samples/Kicks"), kicks);
        assert_eq!(key(user, user), None);
        assert_eq!(key(user, Path::new("/elsewhere")), None);
        assert_eq!(name("My Presets/SuperSaw"), "SuperSaw");
        assert_eq!(sub("My Presets/SuperSaw").as_deref(), Some("My Presets/SuperSaw"));
        assert_eq!(sub("My Presets"), None);
    }
}
