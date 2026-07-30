//! The audience control in the composer. `OMEGA-DELTA-0094`, omega#107.
//!
//! `omega_audience` holds the rules and knows nothing about a window. This is
//! the other half: the durable record, the selection, and the one control a
//! person reads before they type.
//!
//! # Why the label is read from the thread and never from the selection
//!
//! This is the whole point of omega#107, so it is worth stating where somebody
//! editing this file will see it. [`thread_audience_label`] takes a
//! [`ThreadId`] and no selection. There is no parameter it could read one
//! from. If it read the selection instead, then selecting a community audience
//! would repaint every thread already on screen — including the private ones
//! from last week — as belonging to it. Nothing would have been published, and
//! a person looking at their own editor would have no way to learn that.
//!
//! `the_composer_reads_the_audience_from_the_thread` in `omega_deltas` holds
//! that function to the record.
//!
//! # Why there is no modal, and no settings page
//!
//! The owner has rejected that shape repeatedly. The control is a button in the
//! composer row beside the executor line and the model selector, and the
//! current audience is on its face, so the answer to "is what I am typing
//! private" needs no click.
//!
//! # Why the record is written where a thread starts, not where it is drawn
//!
//! [`record_thread_opening`] is called from `ConversationView::new`, which is
//! the one place that knows the difference between a thread that did not exist
//! a moment ago and one being opened again — `resume_session_id` and
//! `thread_id` are both `Option` there. At draw time that difference is not
//! available: `AcpThread::is_draft_thread` is `entries().is_empty()`, which is
//! also true of a resumed thread whose entries have not finished loading, and
//! binding on it would hand a community audience to somebody's old private
//! conversation on a slow disk.

use std::rc::Rc;

use db::kvp::KeyValueStore;
use gpui::{AnyElement, App, Global, Task, TaskExt as _};
use omega_audience::{
    Audience, AudienceBook, AudienceId, AudienceRoster, Reach, SELECTION_MENU_HEADER,
    SWITCHING_DOES_NOT_MOVE_A_THREAD, THREAD_IS_NOT_IN_THE_SELECTION, ThreadAudience,
    ThreadOpening, audience_for_opening,
};
use omega_identity::PublicIdentity;
use ui::{Button, ContextMenu, ContextMenuEntry, PopoverMenu, Tooltip, Window, prelude::*};
use util::ResultExt as _;

use crate::account_scope::AccountScope;
use crate::thread_metadata_store::ThreadId;

/// Where the audience record lives in the key-value store.
const NAMESPACE: &str = "omega_audience";

/// The key holding the whole [`AudienceBook`].
const BOOK_KEY: &str = "thread_audiences";

/// The key holding the audience the next thread starts in.
const SELECTION_KEY: &str = "selected_audience";

/// A roster the owner can look at before omega#108 exists.
///
/// omega#107's acceptance is four rendered windows, and two of them — the
/// selector listing more than Local, and an existing thread not moving when the
/// selection changes — cannot be looked at on a build where the only audience
/// is Local. This environment variable adds one entry so they can be.
///
/// What the fixture is, and what it is forbidden from becoming, is
/// `omega_audience::preview_audience` and the reserved
/// `omega_audience::PREVIEW_PREFIX`. Reading the variable is this module's job
/// because this module is the one that touches ambient state; deciding what the
/// value means is not, and was moved so it could be tested on a machine that is
/// not in the right state.
pub use omega_audience::PREVIEW_ENV_VAR;

/// The selection, the roster, and every thread's recorded audience.
#[derive(Default)]
struct OmegaAudience {
    /// `None` until the first read, so this costs nothing on a launch that
    /// never opens a thread.
    loaded: Option<Loaded>,
}

/// Cheap to clone on purpose.
///
/// Everything here is read on every draw of the composer, and the book grows
/// with the number of threads on the machine. Behind an `Rc` a draw copies a
/// pointer; by value it would copy every binding twice a frame.
#[derive(Clone)]
struct Loaded {
    scope: AccountScope,
    roster: Rc<AudienceRoster>,
    selected: AudienceId,
    book: Rc<AudienceBook<String>>,
}

impl Global for OmegaAudience {}

fn read_scoped_or_migrate(
    scope: &AccountScope,
    key: &'static str,
    target_key: String,
    cx: &App,
) -> Option<String> {
    let store = KeyValueStore::global(cx);
    let namespace = scope.namespace(NAMESPACE);
    if let Some(value) = store
        .scoped(&namespace)
        .read(&target_key)
        .log_err()
        .flatten()
    {
        return Some(value);
    }
    let value = store.scoped(NAMESPACE).read(key).log_err().flatten()?;
    let migration_store = store;
    let migration_scope = scope.clone();
    let migration_value = value.clone();
    cx.background_spawn(async move {
        migration_scope.ensure_current()?;
        let target = migration_store.scoped(&namespace);
        target
            .write(target_key.clone(), migration_value.clone())
            .await?;
        if let Err(stale) = migration_scope.ensure_current() {
            if migration_scope.is_purge_barrier_active()? {
                target.delete_all().await?;
            }
            return Err(stale);
        }
        anyhow::ensure!(
            target.read(&target_key)?.as_deref() == Some(migration_value.as_str()),
            "the migrated audience value could not be read back"
        );
        migration_store
            .scoped(NAMESPACE)
            .delete(key.to_string())
            .await
    })
    .detach_and_log_err(cx);
    Some(value)
}

/// Everything the control needs, hydrated from the key-value store once.
fn loaded(cx: &mut App) -> Loaded {
    let scope = AccountScope::observed();
    if let Some(loaded) = cx
        .default_global::<OmegaAudience>()
        .loaded
        .clone()
        .filter(|loaded| loaded.scope == scope)
    {
        return loaded;
    }

    let book: AudienceBook<String> =
        read_scoped_or_migrate(&scope, BOOK_KEY, scope.profile_key(BOOK_KEY), cx)
            .and_then(|raw| serde_json::from_str(&raw).log_err())
            .unwrap_or_default();

    // `OMEGA-DELTA-0113`, omega#108. The rooms this profile has joined, then
    // the fixture if the environment asked for it. Real places first: the
    // fixture is a rendering aid and an entry above somebody's actual workspace
    // would be the fixture presenting itself as the more important of the two.
    //
    // Read through the seam rather than built here, because what a joined room
    // *is* — a Forge repository, a membership the Forge granted, a refusal when
    // it did not — belongs in `omega_community`, and this module's job is the
    // window.
    let roster = AudienceRoster::new(
        crate::omega_community_control::joined_audiences(cx)
            .into_iter()
            .chain(preview_audience()),
    );

    // A selection that names an audience this profile no longer has resolves
    // to Local rather than staying pointed at it. Leaving a community and
    // finding the composer still offering to start threads in it would be a
    // control describing a door that is not there.
    let selected =
        read_scoped_or_migrate(&scope, SELECTION_KEY, scope.pending_key(SELECTION_KEY), cx)
            .map(|raw| AudienceId::from_key(&raw))
            .filter(|id| roster.resolve(id).is_some())
            .unwrap_or_else(AudienceId::local);

    let loaded = Loaded {
        scope,
        roster: Rc::new(roster),
        selected,
        book: Rc::new(book),
    };
    cx.default_global::<OmegaAudience>().loaded = Some(loaded.clone());
    loaded
}

/// Drops the cached roster, selection and book so the next read rebuilds them.
///
/// `OMEGA-DELTA-0113`, omega#108. Joining or leaving a room changes what the
/// selector offers, and the roster is hydrated once and held in a global.
/// Without this, a person who joins a room in the conversation sees the
/// composer keep offering the list it had at launch, which reads as the join
/// not having worked.
///
/// It drops the whole `Loaded`, not just the roster, because the selection is
/// filtered against the roster when it is read: leaving the room a person had
/// selected has to fall back to Local, and that decision is made in
/// [`loaded`].
pub fn forget_roster(cx: &mut App) {
    cx.default_global::<OmegaAudience>().loaded = None;
    cx.refresh_windows();
}

pub fn purge_account(identity: &PublicIdentity, cx: &App) -> Task<anyhow::Result<()>> {
    let store = KeyValueStore::global(cx);
    let namespace = AccountScope::namespace_for_identity(NAMESPACE, identity);
    cx.background_spawn(async move {
        let scoped = store.scoped(&namespace);
        scoped.delete_all().await?;
        anyhow::ensure!(
            scoped.read(BOOK_KEY)?.is_none(),
            "the account audience records remained after purge"
        );
        Ok(())
    })
}

/// The fixture audience, when the environment asks for it.
///
/// Reads the variable and decides nothing. What an absent, empty, `0`, `1` or
/// named value means is `omega_audience::preview_audience`, which takes the
/// value as a parameter and is checked there.
fn preview_audience() -> Option<Audience> {
    omega_audience::preview_audience(std::env::var(PREVIEW_ENV_VAR).ok().as_deref())
}

fn persist_book(scope: AccountScope, book: &AudienceBook<String>, cx: &App) {
    let store = KeyValueStore::global(cx);
    let namespace = scope.namespace(NAMESPACE);
    let key = scope.profile_key(BOOK_KEY);
    let Some(payload) = serde_json::to_string(book).log_err() else {
        return;
    };
    cx.background_spawn(async move {
        scope.ensure_current()?;
        store.scoped(&namespace).write(key, payload).await?;
        if let Err(stale) = scope.ensure_current() {
            if scope.is_purge_barrier_active()? {
                store.scoped(&namespace).delete_all().await?;
            }
            return Err(stale);
        }
        Ok(())
    })
    .detach_and_log_err(cx);
}

fn persist_selection(scope: AccountScope, selected: &AudienceId, cx: &App) {
    let store = KeyValueStore::global(cx);
    let namespace = scope.namespace(NAMESPACE);
    let key = scope.pending_key(SELECTION_KEY);
    let payload = selected.as_key().to_string();
    cx.background_spawn(async move {
        scope.ensure_current()?;
        store.scoped(&namespace).write(key, payload).await?;
        if let Err(stale) = scope.ensure_current() {
            if scope.is_purge_barrier_active()? {
                store.scoped(&namespace).delete_all().await?;
            }
            return Err(stale);
        }
        Ok(())
    })
    .detach_and_log_err(cx);
}

/// Record the audience a thread belongs to, once, when it opens.
///
/// A thread that already has a record keeps it, whatever is selected — that is
/// omega#107 deliverable 5, and it is why this returns early rather than
/// calling `bind` and ignoring the refusal.
pub fn record_thread_opening(thread_id: ThreadId, opening: ThreadOpening, cx: &mut App) {
    let mut loaded = loaded(cx);
    let key = thread_id.to_key_string();

    if loaded.book.recorded(&key).is_some() {
        return;
    }

    let audience = audience_for_opening(None, &loaded.selected, opening);

    // A local thread is not written down, because an absent record already
    // means Local — `AudienceBook::audience_of` resolves a thread it has never
    // seen to `Audience::local`, and it does so whatever is selected. Writing
    // the row would change no answer this module can give.
    //
    // It would change two other things, both bad. It puts a durable write on
    // the path of every thread on a machine that has joined nothing, which is
    // every machine until omega#108 — `omega_audience` is kept free of a
    // socket precisely so that Local costs nothing, and a disk write per thread
    // is not nothing. And it is a background write inside
    // `ConversationView::new`: adding one broke
    // `test_select_agent_action_updates_visible_draft`, which persists the
    // last-used agent through the same key-value store and reads it back after
    // `run_until_parked`. That test was right to fail. Local is the path that
    // has to keep working when everything else is down, and the way to keep it
    // that way is to have it do less, not to make the other work wait for it.
    if audience.is_local() {
        return;
    }

    if Rc::make_mut(&mut loaded.book)
        .bind(key, audience)
        .log_err()
        .is_none()
    {
        return;
    }

    persist_book(loaded.scope.clone(), &loaded.book, cx);
    cx.default_global::<OmegaAudience>().loaded = Some(loaded);
}

/// The audience a thread belongs to.
///
/// Takes no selection, on purpose. See the module comment.
pub fn thread_audience(thread_id: ThreadId, cx: &mut App) -> ThreadAudience {
    let loaded = loaded(cx);
    loaded
        .roster
        .describe(&loaded.book.audience_of(&thread_id.to_key_string()))
}

/// What the composer writes on the control's face.
///
/// Pinned by `the_composer_reads_the_audience_from_the_thread` in
/// `omega_deltas`: this function names the thread and never the selection.
pub fn thread_audience_label(thread_id: ThreadId, cx: &mut App) -> SharedString {
    SharedString::from(thread_audience(thread_id, cx).label())
}

/// The audience the next thread starts in.
pub fn selected_audience(cx: &mut App) -> AudienceId {
    loaded(cx).selected
}

/// Choose the audience the next thread starts in.
///
/// Deliberately does not touch `thread_id` anywhere: choosing does not move a
/// thread. The menu says so in as many words, because a control that appears to
/// do nothing is worse than one that explains itself.
pub fn select_audience(audience: AudienceId, cx: &mut App) {
    let mut loaded = loaded(cx);
    if loaded.selected == audience {
        return;
    }
    loaded.selected = audience;
    persist_selection(loaded.scope.clone(), &loaded.selected, cx);
    cx.default_global::<OmegaAudience>().loaded = Some(loaded);
    cx.refresh_windows();
}

/// The icon that stands for a reach.
fn reach_icon(audience: &ThreadAudience) -> IconName {
    match audience {
        ThreadAudience::Known(audience) => match audience.reach() {
            Reach::ThisComputer => IconName::Lock,
            Reach::Shared => IconName::Public,
        },
        // Not a lock. Omega cannot say this thread is private, and an icon that
        // implies it would be the confident wrong answer.
        ThreadAudience::Unresolved(_) => IconName::Warning,
    }
}

/// The composer's audience control.
///
/// The current audience is on the face of the button, so acceptance 4 — "a
/// person must never have to guess whether what they type is private" — is
/// satisfied without opening anything.
pub fn render_audience_control(thread_id: ThreadId, cx: &mut App) -> AnyElement {
    let audience = thread_audience(thread_id, cx);
    let label = thread_audience_label(thread_id, cx);
    let description = SharedString::from(audience.description());
    let icon = reach_icon(&audience);
    let entries: Vec<Audience> = loaded(cx).roster.entries().collect();
    let selected = selected_audience(cx);
    let selection_differs = selected != *thread_audience_id(&audience);

    let trigger = Button::new("omega-audience-selector", label)
        .label_size(LabelSize::XSmall)
        .color(Color::Muted)
        .start_icon(Icon::new(icon).size(IconSize::XSmall).color(Color::Muted))
        .end_icon(
            Icon::new(IconName::ChevronDown)
                .size(IconSize::XSmall)
                .color(Color::Muted),
        );

    PopoverMenu::new("omega-audience")
        .trigger_with_tooltip(
            trigger,
            Tooltip::element(move |_window, _cx| {
                Label::new(description.clone())
                    .size(LabelSize::Small)
                    .into_any_element()
            }),
        )
        .anchor(gpui::Anchor::BottomLeft)
        .menu(move |window, cx| {
            Some(build_menu(
                entries.clone(),
                selected.clone(),
                selection_differs,
                window,
                cx,
            ))
        })
        .into_any_element()
}

/// The identity behind a described audience, resolved or not.
fn thread_audience_id(audience: &ThreadAudience) -> &AudienceId {
    match audience {
        ThreadAudience::Known(audience) => audience.id(),
        ThreadAudience::Unresolved(id) => id,
    }
}

fn build_menu(
    entries: Vec<Audience>,
    selected: AudienceId,
    selection_differs: bool,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Entity<ContextMenu> {
    ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
        menu = menu.header(SELECTION_MENU_HEADER);

        for audience in entries.clone() {
            let is_selected = audience.id() == &selected;
            let id = audience.id().clone();
            let description = SharedString::from(audience.description());
            menu.push_item(
                ContextMenuEntry::new(SharedString::from(audience.name().to_string()))
                    .icon(match audience.reach() {
                        Reach::ThisComputer => IconName::Lock,
                        Reach::Shared => IconName::Public,
                    })
                    .icon_size(IconSize::XSmall)
                    .toggleable(IconPosition::End, is_selected)
                    .documentation_aside(ui::DocumentationSide::Left, move |_| {
                        Label::new(description.clone()).into_any_element()
                    })
                    .handler(move |_window, cx| {
                        select_audience(id.clone(), cx);
                    }),
            );
        }

        // omega#107 deliverable 5, said out loud. Choosing here changes the
        // next thread, not this one, and a person who picks an audience and
        // watches the button not change is entitled to know why rather than to
        // conclude the control is broken.
        //
        // These two sentences are the least verified thing in this feature and
        // they are the one part of it that cannot be checked at all — whether
        // they land needs a window and somebody who has not read this file. So
        // they live in `omega_audience` beside the rule they describe, with
        // the guess and its falsifier written out, and
        // `the_menus_sentences_are_written_once` fails if a literal reappears
        // here. Changing the wording is one edit, in one place.
        menu = menu.separator().custom_row(move |_window, _cx| {
            v_flex()
                .max_w_64()
                .child(
                    Label::new(SWITCHING_DOES_NOT_MOVE_A_THREAD)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .when(selection_differs, |this| {
                    this.child(
                        h_flex().child(
                            Label::new(THREAD_IS_NOT_IN_THE_SELECTION)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                    )
                })
                .into_any_element()
        });

        menu.key_context("OmegaAudienceSelector")
    })
}
