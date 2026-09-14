// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared plugin inventory primitives.
//!
//! Every plugin dimension (`Packager`, `ForgeSource`, `BuildSystem`,
//! `RegistrySource`, `ArtifactFormat`, `Signer`, `DependencyMapper`,
//! `PackageIndex`) is a **static, order-sensitive list of named, stateless
//! trait objects**. Each dimension used to hand-write the same three
//! functions around that list — `all_*()`, `get_*(name)`, `*_names()` — and
//! each trait re-declared `name()`/`description()`.
//!
//! This module holds the two pieces they share:
//!
//! * [`Plugin`] — the `name`/`description` identity every dimension exposes.
//! * [`PluginSet`] — the registry over such a list: `get`/`names`/`first`/
//!   `take`/`take_first`, so a dimension's free functions become one-liners.
//!
//! Registration stays static and explicit (no dynamic loading); a dimension
//! builds its `Vec<Box<dyn _>>` and wraps it in a `PluginSet`.
//!
//! ## Converting a dimension
//!
//! 1. Make the dimension trait extend [`Plugin`] and delete its `name`/
//!    `description` methods (they now come from the supertrait).
//! 2. In each impl file, drop those two methods and add
//!    [`plugin_identity!`] instead.
//! 3. Rewrite `all_*`/`get_*`/`*_names` as one-liners over [`PluginSet`].
//!
//! Not-yet-converted dimensions are unaffected: nothing here requires them to
//! opt in, so the migration can proceed one dimension at a time.

/// Identity shared by every plugin dimension.
pub trait Plugin: Send + Sync {
    /// Canonical name — the value accepted in `package.yaml` / on the CLI
    /// (`package_format:`, `--format`, `source:`, `build_system:` …).
    fn name(&self) -> &'static str;

    /// Human-readable description for help and error text.
    fn description(&self) -> &'static str;
}

/// A static, ordered list of named plugins.
///
/// The order is **load-bearing**: dimensions that auto-detect or resolve
/// (artifact formats, signers, build systems) rely on it being
/// most-specific → catch-all, and `first`/`take_first` preserve it.
pub struct PluginSet<T: Plugin + ?Sized> {
    items: Vec<Box<T>>,
}

impl<T: Plugin + ?Sized> PluginSet<T> {
    /// Wrap a dimension's freshly-built plugin list.
    pub fn new(items: Vec<Box<T>>) -> Self {
        Self { items }
    }

    /// Borrowing iterator in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter().map(|b| &**b)
    }

    /// First plugin matching `pred`, in registration order.
    pub fn first(&self, pred: impl Fn(&T) -> bool) -> Option<&T> {
        self.iter().find(|p| pred(p))
    }

    /// Look up by canonical name, case-insensitively.
    pub fn get(&self, name: &str) -> Option<&T> {
        let want = normalize(name);
        self.iter().find(|p| p.name() == want)
    }

    /// Canonical names, in registration order.
    pub fn names(&self) -> Vec<&'static str> {
        self.iter().map(|p| p.name()).collect()
    }

    /// Consuming lookup by canonical name. Matches the `Option<Box<dyn _>>`
    /// signature of the existing `get_*` functions.
    pub fn take(self, name: &str) -> Option<Box<T>> {
        let want = normalize(name);
        self.take_first(|p| p.name() == want)
    }

    /// Consuming first-match lookup (for order-sensitive resolution).
    pub fn take_first(mut self, pred: impl Fn(&T) -> bool) -> Option<Box<T>> {
        let idx = self.items.iter().position(|p| pred(p))?;
        Some(self.items.swap_remove(idx))
    }
}

/// Normalize a user-supplied plugin name for case-insensitive lookup.
fn normalize(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Implement [`Plugin`] for a plugin type from its identity, so a dimension
/// impl only carries its *behaviour*.
///
/// ```ignore
/// plugin_identity!(ApkRsa, "apk-rsa", "Alpine apk v2 RSA/SHA-1 signature");
/// ```
macro_rules! plugin_identity {
    ($ty:ty, $name:expr, $desc:expr) => {
        impl $crate::plugins::plugin::Plugin for $ty {
            fn name(&self) -> &'static str {
                $name
            }

            fn description(&self) -> &'static str {
                $desc
            }
        }
    };
}

pub(crate) use plugin_identity;
