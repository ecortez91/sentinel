//! Plugin registry: manages all registered plugins.
//!
//! Single Responsibility: only handles registration and lookup.
//! The app owns the registry and delegates to individual plugins.

use super::Plugin;

/// Central registry of all compiled-in plugins.
#[allow(dead_code)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

#[allow(dead_code)]
impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin. Plugins are rendered as tabs in registration order.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Get all plugins (immutable).
    pub fn plugins(&self) -> &[Box<dyn Plugin>] {
        &self.plugins
    }

    /// Get all plugins (mutable) for tick/key handling.
    pub fn plugins_mut(&mut self) -> &mut [Box<dyn Plugin>] {
        &mut self.plugins
    }

    /// Iterator over enabled plugins only.
    pub fn enabled_plugins(&self) -> impl Iterator<Item = (usize, &dyn Plugin)> {
        self.plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_enabled())
            .map(|(i, p)| (i, p.as_ref()))
    }

    /// Count of enabled plugins.
    pub fn enabled_count(&self) -> usize {
        self.plugins.iter().filter(|p| p.is_enabled()).count()
    }

    /// Get a plugin by index (mutable).
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Box<dyn Plugin>> {
        self.plugins.get_mut(index)
    }

    /// Get a plugin by index (immutable).
    pub fn get(&self, index: usize) -> Option<&Box<dyn Plugin>> {
        self.plugins.get(index)
    }

    /// Find a plugin by ID.
    pub fn find_by_id(&self, id: &str) -> Option<(usize, &dyn Plugin)> {
        self.plugins
            .iter()
            .enumerate()
            .find(|(_, p)| p.id() == id)
            .map(|(i, p)| (i, p.as_ref()))
    }

    /// Find a plugin by ID (mutable).
    pub fn find_by_id_mut(&mut self, id: &str) -> Option<(usize, &mut Box<dyn Plugin>)> {
        self.plugins
            .iter_mut()
            .enumerate()
            .find(|(_, p)| p.id() == id)
    }

    /// Tick all plugins (drain channels, update state).
    pub fn tick_all(&mut self) {
        for plugin in &mut self.plugins {
            plugin.tick();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;
    use ratatui::{layout::Rect, Frame};

    struct FakePlugin {
        name: &'static str,
        enabled: bool,
    }

    impl Plugin for FakePlugin {
        fn id(&self) -> &str {
            self.name
        }
        fn tab_label(&self) -> &str {
            self.name
        }
        fn is_enabled(&self) -> bool {
            self.enabled
        }
        fn render(&self, _: &mut Frame, _: Rect, _: &Theme, _: &crate::ui::glyphs::Glyphs) {}
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(FakePlugin {
            name: "alpha",
            enabled: true,
        }));
        reg.register(Box::new(FakePlugin {
            name: "beta",
            enabled: false,
        }));
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.enabled_count(), 1);
    }

    #[test]
    fn find_by_id_works() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(FakePlugin {
            name: "market",
            enabled: true,
        }));
        assert!(reg.find_by_id("market").is_some());
        assert!(reg.find_by_id("unknown").is_none());
    }

    #[test]
    fn enabled_plugins_filters_correctly() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(FakePlugin {
            name: "a",
            enabled: true,
        }));
        reg.register(Box::new(FakePlugin {
            name: "b",
            enabled: false,
        }));
        reg.register(Box::new(FakePlugin {
            name: "c",
            enabled: true,
        }));
        let enabled: Vec<_> = reg.enabled_plugins().collect();
        assert_eq!(enabled.len(), 2);
        assert_eq!(enabled[0].1.id(), "a");
        assert_eq!(enabled[1].1.id(), "c");
    }
}
