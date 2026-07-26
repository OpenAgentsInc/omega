//! Provides hooks for customizing the behavior of the command palette.

#![deny(missing_docs)]

use std::{any::TypeId, rc::Rc};

use collections::{HashSet, TypeIdHashSet};
use derive_more::{Deref, DerefMut};
use gpui::{Action, App, BorrowAppContext, Global, Task, WeakEntity};
use workspace::Workspace;

/// Initializes the command palette hooks.
pub fn init(cx: &mut App) {
    cx.set_global(GlobalCommandPaletteFilter::default());
}

/// A filter for the command palette.
#[derive(Default)]
pub struct CommandPaletteFilter {
    hidden_namespaces: HashSet<&'static str>,
    hidden_action_types: TypeIdHashSet,
    /// Actions that have explicitly been shown. These should be shown even if
    /// they are in a hidden namespace.
    shown_action_types: TypeIdHashSet,
    /// The admitted set, when the palette is restricted to one.
    ///
    /// `hide_namespace` is a denylist, and a denylist cannot express "only
    /// these": Omega registers actions in more than sixty namespaces, and a
    /// mode that hid every namespace it knew about would keep admitting the
    /// ones a later crate adds. `Some` here inverts the question — anything
    /// outside the two lists is hidden. Both lists are `&'static`, so the
    /// admitted set is a constant some other code points at rather than a list
    /// assembled at runtime.
    restriction: Option<Restriction>,
}

/// The admitted set of a restricted command palette.
#[derive(Clone, Copy)]
struct Restriction {
    namespaces: &'static [&'static str],
    actions: &'static [&'static str],
}

#[derive(Deref, DerefMut, Default)]
struct GlobalCommandPaletteFilter(CommandPaletteFilter);

impl Global for GlobalCommandPaletteFilter {}

impl CommandPaletteFilter {
    /// Returns the global [`CommandPaletteFilter`], if one is set.
    pub fn try_global(cx: &App) -> Option<&CommandPaletteFilter> {
        cx.try_global::<GlobalCommandPaletteFilter>()
            .map(|filter| &filter.0)
    }

    /// Returns a mutable reference to the global [`CommandPaletteFilter`].
    pub fn global_mut(cx: &mut App) -> &mut Self {
        cx.global_mut::<GlobalCommandPaletteFilter>()
    }

    /// Updates the global [`CommandPaletteFilter`] using the given closure.
    pub fn update_global<F>(cx: &mut App, update: F)
    where
        F: FnOnce(&mut Self, &mut App),
    {
        if cx.has_global::<GlobalCommandPaletteFilter>() {
            cx.update_global(|this: &mut GlobalCommandPaletteFilter, cx| update(&mut this.0, cx))
        }
    }

    /// Returns whether the given [`Action`] is hidden by the filter.
    pub fn is_hidden(&self, action: &dyn Action) -> bool {
        let name = action.name();
        let namespace = name.split("::").next().unwrap_or("malformed action name");

        // If this action has specifically been shown then it should be visible.
        if self.shown_action_types.contains(&action.type_id()) {
            return false;
        }

        if let Some(restriction) = self.restriction
            && !restriction.namespaces.contains(&namespace)
            && !restriction.actions.contains(&name)
        {
            return true;
        }

        self.hidden_namespaces.contains(namespace)
            || self.hidden_action_types.contains(&action.type_id())
    }

    /// Hides every action outside the given namespaces and action names.
    ///
    /// The inverse of [`Self::hide_namespace`], for a mode that admits a short
    /// list rather than denying a long one. The palette still opens, and it
    /// lists what the restriction admits.
    pub fn restrict_to(
        &mut self,
        namespaces: &'static [&'static str],
        actions: &'static [&'static str],
    ) {
        self.restriction = Some(Restriction {
            namespaces,
            actions,
        });
    }

    /// Removes the restriction set by [`Self::restrict_to`], if any.
    pub fn clear_restriction(&mut self) {
        self.restriction = None;
    }

    /// Is the palette currently restricted to an admitted set?
    pub fn is_restricted(&self) -> bool {
        self.restriction.is_some()
    }

    /// Hides all actions in the given namespace.
    pub fn hide_namespace(&mut self, namespace: &'static str) {
        self.hidden_namespaces.insert(namespace);
    }

    /// Shows all actions in the given namespace.
    pub fn show_namespace(&mut self, namespace: &'static str) {
        self.hidden_namespaces.remove(namespace);
    }

    /// Hides all actions with the given types.
    pub fn hide_action_types<'a>(&mut self, action_types: impl IntoIterator<Item = &'a TypeId>) {
        for action_type in action_types {
            self.hidden_action_types.insert(*action_type);
            self.shown_action_types.remove(action_type);
        }
    }

    /// Shows all actions with the given types.
    pub fn show_action_types<'a>(&mut self, action_types: impl IntoIterator<Item = &'a TypeId>) {
        for action_type in action_types {
            self.shown_action_types.insert(*action_type);
            self.hidden_action_types.remove(action_type);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::actions;

    actions!(
        command_palette_hooks_test,
        [
            /// Stands in for an action zero base admits.
            Admitted,
            /// Stands in for an action zero base hides.
            Hidden
        ]
    );

    /// omega#99, `OMEGA-DELTA-0048`. A restricted palette still opens, and it
    /// lists what the restriction admits and nothing else.
    ///
    /// The denylist could not express this. Omega registers actions in more
    /// than sixty namespaces, so a mode that hid every namespace it knew about
    /// would keep admitting the ones a later crate adds — the surface would
    /// grow back one release at a time and nothing would fail.
    #[test]
    fn a_restriction_admits_only_what_it_names() {
        const NAMESPACES: &[&str] = &["command_palette_hooks_test"];
        const NONE: &[&str] = &[];

        let mut filter = CommandPaletteFilter::default();
        assert!(!filter.is_restricted());
        assert!(!filter.is_hidden(&Admitted));
        assert!(!filter.is_hidden(&Hidden));

        // A restriction naming no namespace at all hides both.
        filter.restrict_to(NONE, NONE);
        assert!(filter.is_restricted());
        assert!(filter.is_hidden(&Admitted));
        assert!(filter.is_hidden(&Hidden));

        // Naming the namespace admits both; naming one action admits one.
        filter.restrict_to(NAMESPACES, NONE);
        assert!(!filter.is_hidden(&Admitted));
        assert!(!filter.is_hidden(&Hidden));

        filter.restrict_to(NONE, &["command_palette_hooks_test::Admitted"]);
        assert!(!filter.is_hidden(&Admitted));
        assert!(filter.is_hidden(&Hidden));

        // The denylist keeps working underneath, so a settings change that
        // hides a namespace still hides it inside the mode.
        filter.restrict_to(NAMESPACES, NONE);
        filter.hide_namespace("command_palette_hooks_test");
        assert!(filter.is_hidden(&Admitted));

        filter.show_namespace("command_palette_hooks_test");
        filter.clear_restriction();
        assert!(!filter.is_restricted());
        assert!(!filter.is_hidden(&Admitted));
        assert!(!filter.is_hidden(&Hidden));
    }
}

/// The result of intercepting a command palette command.
#[derive(Debug)]
pub struct CommandInterceptItem {
    /// The action produced as a result of the interception.
    pub action: Box<dyn Action>,
    /// The display string to show in the command palette for this result.
    pub string: String,
    /// The character positions in the string that match the query.
    /// Used for highlighting matched characters in the command palette UI.
    pub positions: Vec<usize>,
}

/// The result of intercepting a command palette command.
#[derive(Default, Debug)]
pub struct CommandInterceptResult {
    /// The items
    pub results: Vec<CommandInterceptItem>,
    /// Whether or not to continue to show the normal matches
    pub exclusive: bool,
}

/// An interceptor for the command palette.
#[derive(Clone)]
pub struct GlobalCommandPaletteInterceptor(
    Rc<dyn Fn(&str, WeakEntity<Workspace>, &mut App) -> Task<CommandInterceptResult>>,
);

impl Global for GlobalCommandPaletteInterceptor {}

impl GlobalCommandPaletteInterceptor {
    /// Sets the global interceptor.
    ///
    /// This will override the previous interceptor, if it exists.
    pub fn set(
        cx: &mut App,
        interceptor: impl Fn(&str, WeakEntity<Workspace>, &mut App) -> Task<CommandInterceptResult>
        + 'static,
    ) {
        cx.set_global(Self(Rc::new(interceptor)));
    }

    /// Clears the global interceptor.
    pub fn clear(cx: &mut App) {
        if cx.has_global::<Self>() {
            cx.remove_global::<Self>();
        }
    }

    /// Intercepts the given query from the command palette.
    pub fn intercept(
        query: &str,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) -> Option<Task<CommandInterceptResult>> {
        let interceptor = cx.try_global::<Self>()?;
        let handler = interceptor.0.clone();
        Some(handler(query, workspace, cx))
    }
}
