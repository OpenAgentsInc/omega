use crate::{DbThread, DbThreadMetadata, ThreadEventSequence, ThreadsDatabase};
use agent_client_protocol::schema::v1 as acp;
use anyhow::{Result, anyhow};
use futures::{FutureExt, future::Shared};
use gpui::{App, Context, Entity, Global, Task, prelude::*};
use util::path_list::PathList;

struct GlobalThreadStore(Entity<ThreadStore>);

impl Global for GlobalThreadStore {}

pub struct ThreadStore {
    threads: Vec<DbThreadMetadata>,
    reload_task: Shared<Task<()>>,
}

impl ThreadStore {
    pub fn init_global(cx: &mut App) {
        let thread_store = cx.new(|cx| Self::new(cx));
        cx.set_global(GlobalThreadStore(thread_store));
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalThreadStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalThreadStore>().map(|g| g.0.clone())
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let reload_task = Self::spawn_reload(cx);
        Self {
            threads: Vec::new(),
            reload_task,
        }
    }

    /// Resolves when the most recently initiated reload has completed.
    /// Callers that need to read `entries()` and can't tolerate the initial
    /// empty state must await this before reading.
    pub fn reload_task(&self) -> Shared<Task<()>> {
        self.reload_task.clone()
    }

    pub fn thread_from_session_id(&self, session_id: &acp::SessionId) -> Option<&DbThreadMetadata> {
        self.threads.iter().find(|thread| &thread.id == session_id)
    }

    pub fn load_thread(
        &mut self,
        id: acp::SessionId,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<DbThread>>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.background_spawn(async move {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            database.load_thread(id).await
        })
    }

    pub fn load_thread_at(
        &mut self,
        id: acp::SessionId,
        event_sequence: ThreadEventSequence,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<DbThread>>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.background_spawn(async move {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            let Some(mut thread) = database.load_thread(id).await? else {
                return Ok(None);
            };
            thread.prepare_for_resume(Some(event_sequence))?;
            Ok(Some(thread))
        })
    }

    pub fn load_thread_at_message_index(
        &mut self,
        id: acp::SessionId,
        message_index: usize,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<DbThread>>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.background_spawn(async move {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            let Some(mut thread) = database.load_thread(id).await? else {
                return Ok(None);
            };
            thread.prepare_for_resume_at_message_index(message_index)?;
            Ok(Some(thread))
        })
    }

    pub fn select_thread_at(
        &mut self,
        id: acp::SessionId,
        event_sequence: ThreadEventSequence,
        cx: &mut Context<Self>,
    ) -> Task<Result<bool>> {
        let folder_paths = self
            .thread_from_session_id(&id)
            .map(|metadata| metadata.folder_paths.clone())
            .unwrap_or_default();
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|error| anyhow!(error))?;
            let Some(mut thread) = database.load_thread(id.clone()).await? else {
                return Ok(false);
            };
            thread.prepare_for_resume(Some(event_sequence))?;
            database.save_thread(id, thread, folder_paths).await?;
            this.update(cx, |this, cx| this.reload(cx))?;
            Ok(true)
        })
    }

    pub fn select_thread_at_message_index(
        &mut self,
        id: acp::SessionId,
        message_index: usize,
        cx: &mut Context<Self>,
    ) -> Task<Result<bool>> {
        let folder_paths = self
            .thread_from_session_id(&id)
            .map(|metadata| metadata.folder_paths.clone())
            .unwrap_or_default();
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|error| anyhow!(error))?;
            let Some(mut thread) = database.load_thread(id.clone()).await? else {
                return Ok(false);
            };
            thread.prepare_for_resume_at_message_index(message_index)?;
            database.save_thread(id, thread, folder_paths).await?;
            this.update(cx, |this, cx| this.reload(cx))?;
            Ok(true)
        })
    }

    pub fn fork_thread(
        &mut self,
        source_id: acp::SessionId,
        event_sequence: ThreadEventSequence,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<acp::SessionId>>> {
        let folder_paths = self
            .thread_from_session_id(&source_id)
            .map(|metadata| metadata.folder_paths.clone())
            .unwrap_or_default();
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            let Some(source) = database.load_thread(source_id.clone()).await? else {
                return Ok(None);
            };
            let fork = source.fork_at(source_id, event_sequence)?;
            let fork_id = acp::SessionId::new(uuid::Uuid::new_v4().to_string());
            database
                .save_thread(fork_id.clone(), fork, folder_paths)
                .await?;
            this.update(cx, |this, cx| this.reload(cx))?;
            Ok(Some(fork_id))
        })
    }

    pub fn fork_thread_at_message_index(
        &mut self,
        source_id: acp::SessionId,
        message_index: usize,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<acp::SessionId>>> {
        let folder_paths = self
            .thread_from_session_id(&source_id)
            .map(|metadata| metadata.folder_paths.clone())
            .unwrap_or_default();
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            let Some(source) = database.load_thread(source_id.clone()).await? else {
                return Ok(None);
            };
            let fork = source.fork_at_message_index(source_id, message_index)?;
            let fork_id = acp::SessionId::new(uuid::Uuid::new_v4().to_string());
            database
                .save_thread(fork_id.clone(), fork, folder_paths)
                .await?;
            this.update(cx, |this, cx| this.reload(cx))?;
            Ok(Some(fork_id))
        })
    }

    pub fn save_thread(
        &mut self,
        id: acp::SessionId,
        thread: crate::DbThread,
        folder_paths: PathList,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            database.save_thread(id, thread, folder_paths).await?;
            this.update(cx, |this, cx| this.reload(cx))
        })
    }

    pub fn delete_thread(
        &mut self,
        id: acp::SessionId,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            database.delete_thread(id.clone()).await?;
            this.update(cx, |this, cx| this.reload(cx))
        })
    }

    pub fn delete_threads(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            database.delete_threads().await?;
            this.update(cx, |this, cx| this.reload(cx))
        })
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_task = Self::spawn_reload(cx);
    }

    fn spawn_reload(cx: &mut Context<Self>) -> Shared<Task<()>> {
        let database_connection = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let Ok(database) = database_connection.await.map_err(|err| anyhow!(err)) else {
                return;
            };
            let Ok(all_threads) = database.list_threads().await else {
                return;
            };
            this.update(cx, |this, cx| {
                this.threads.clear();
                for thread in all_threads {
                    if thread.parent_session_id.is_some() {
                        continue;
                    }
                    this.threads.push(thread);
                }
                cx.notify();
            })
            .ok();
        })
        .shared()
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = DbThreadMetadata> + '_ {
        self.threads.iter().cloned()
    }

    pub fn entry_ids(&self) -> impl Iterator<Item = acp::SessionId> + '_ {
        self.threads.iter().map(|t| t.id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use collections::HashMap;
    use gpui::TestAppContext;
    use std::sync::Arc;

    fn session_id(value: &str) -> acp::SessionId {
        acp::SessionId::new(Arc::<str>::from(value))
    }

    fn make_thread(title: &str, updated_at: DateTime<Utc>) -> DbThread {
        DbThread {
            title: title.to_string().into(),
            messages: Vec::new(),
            updated_at,
            detailed_summary: None,
            initial_project_snapshot: None,
            cumulative_token_usage: Default::default(),
            request_token_usage: HashMap::default(),
            model: None,
            profile: None,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: Default::default(),
            thread_log: Default::default(),
            fork_origin: None,
        }
    }

    fn user_message(text: &str) -> Arc<crate::Message> {
        Arc::new(crate::Message::User(crate::UserMessage {
            id: acp_thread::ClientUserMessageId::new(),
            content: Arc::from([crate::UserMessageContent::Text(text.to_string())]),
        }))
    }

    #[gpui::test]
    async fn load_at_cursor_and_fork_preserve_the_selected_prefix(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();
        let source_id = session_id("source-thread");
        let mut source = make_thread("Source", Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
        let cursor = source.thread_log.append_message(user_message("inherited"));
        source
            .thread_log
            .append_message(user_message("not inherited"));
        source.messages = source.thread_log.messages().unwrap();

        thread_store
            .update(cx, |store, cx| {
                store.save_thread(source_id.clone(), source, PathList::default(), cx)
            })
            .await
            .unwrap();
        cx.run_until_parked();

        let resumed = thread_store
            .update(cx, |store, cx| {
                store.load_thread_at(source_id.clone(), cursor, cx)
            })
            .await
            .unwrap()
            .expect("source thread");
        assert_eq!(resumed.messages.len(), 1);

        assert!(
            thread_store
                .update(cx, |store, cx| {
                    store.select_thread_at(source_id.clone(), cursor, cx)
                })
                .await
                .unwrap()
        );
        let selected = thread_store
            .update(cx, |store, cx| store.load_thread(source_id.clone(), cx))
            .await
            .unwrap()
            .expect("selected source thread");
        assert_eq!(selected.messages.len(), 1);
        assert_eq!(selected.thread_log.active_sequence, Some(cursor));

        let fork_id = thread_store
            .update(cx, |store, cx| {
                store.fork_thread(source_id.clone(), cursor, cx)
            })
            .await
            .unwrap()
            .expect("forked thread");
        assert_ne!(fork_id, source_id);
        let fork = thread_store
            .update(cx, |store, cx| store.load_thread(fork_id, cx))
            .await
            .unwrap()
            .expect("saved fork");
        assert_eq!(fork.messages.len(), 1);
        assert_eq!(
            fork.fork_origin,
            Some(crate::ThreadForkOrigin {
                session_id: source_id,
                event_sequence: cursor,
            })
        );
    }

    #[gpui::test]
    async fn load_and_fork_at_message_index_preserve_the_selected_prefix(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();
        let source_id = session_id("source-thread-by-message");
        let mut source = make_thread("Source", Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
        source.thread_log.append_message(user_message("first"));
        source
            .thread_log
            .append_prompt_cache_layout(crate::PromptCacheLayout {
                system_prompt: "stable prompt".into(),
                tool_order: vec!["terminal".into()],
            });
        let source_sequence = source.thread_log.append_message(user_message("second"));
        source.messages = source.thread_log.messages().unwrap();

        thread_store
            .update(cx, |store, cx| {
                store.save_thread(source_id.clone(), source, PathList::default(), cx)
            })
            .await
            .unwrap();
        cx.run_until_parked();

        let resumed = thread_store
            .update(cx, |store, cx| {
                store.load_thread_at_message_index(source_id.clone(), 0, cx)
            })
            .await
            .unwrap()
            .expect("source thread");
        assert_eq!(resumed.messages.len(), 1);
        assert_eq!(
            resumed
                .thread_log
                .prompt_cache_layout(resumed.thread_log.active_sequence)
                .unwrap(),
            Some(crate::PromptCacheLayout {
                system_prompt: "stable prompt".into(),
                tool_order: vec!["terminal".into()],
            })
        );

        let fork_id = thread_store
            .update(cx, |store, cx| {
                store.fork_thread_at_message_index(source_id.clone(), 0, cx)
            })
            .await
            .unwrap()
            .expect("forked thread");
        let fork = thread_store
            .update(cx, |store, cx| store.load_thread(fork_id, cx))
            .await
            .unwrap()
            .expect("saved fork");
        assert_eq!(fork.messages.len(), 1);
        assert_eq!(
            fork.fork_origin,
            Some(crate::ThreadForkOrigin {
                session_id: source_id,
                event_sequence: source_sequence,
            })
        );
    }

    #[gpui::test]
    async fn test_entries_are_sorted_by_updated_at(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();

        let older_id = session_id("thread-a");
        let newer_id = session_id("thread-b");

        let older_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let newer_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        let save_older = thread_store.update(cx, |store, cx| {
            store.save_thread(older_id.clone(), older_thread, PathList::default(), cx)
        });
        save_older.await.unwrap();

        let save_newer = thread_store.update(cx, |store, cx| {
            store.save_thread(newer_id.clone(), newer_thread, PathList::default(), cx)
        });
        save_newer.await.unwrap();

        cx.run_until_parked();

        let entries: Vec<_> = thread_store.read_with(cx, |store, _cx| store.entries().collect());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, newer_id);
        assert_eq!(entries[1].id, older_id);
    }

    #[gpui::test]
    async fn test_delete_threads_clears_entries(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();

        let thread_id = session_id("thread-a");
        let thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        let save_task = thread_store.update(cx, |store, cx| {
            store.save_thread(thread_id, thread, PathList::default(), cx)
        });
        save_task.await.unwrap();

        cx.run_until_parked();
        assert!(!thread_store.read_with(cx, |store, _cx| store.is_empty()));

        let delete_task = thread_store.update(cx, |store, cx| store.delete_threads(cx));
        delete_task.await.unwrap();
        cx.run_until_parked();

        assert!(thread_store.read_with(cx, |store, _cx| store.is_empty()));
    }

    #[gpui::test]
    async fn test_delete_thread_removes_only_target(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();

        let first_id = session_id("thread-a");
        let second_id = session_id("thread-b");

        let first_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let second_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        let save_first = thread_store.update(cx, |store, cx| {
            store.save_thread(first_id.clone(), first_thread, PathList::default(), cx)
        });
        save_first.await.unwrap();
        let save_second = thread_store.update(cx, |store, cx| {
            store.save_thread(second_id.clone(), second_thread, PathList::default(), cx)
        });
        save_second.await.unwrap();
        cx.run_until_parked();

        let delete_task =
            thread_store.update(cx, |store, cx| store.delete_thread(first_id.clone(), cx));
        delete_task.await.unwrap();
        cx.run_until_parked();

        let entries: Vec<_> = thread_store.read_with(cx, |store, _cx| store.entries().collect());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, second_id);
    }

    #[gpui::test]
    async fn test_save_thread_refreshes_ordering(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();

        let first_id = session_id("thread-a");
        let second_id = session_id("thread-b");

        let first_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let second_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        let save_first = thread_store.update(cx, |store, cx| {
            store.save_thread(first_id.clone(), first_thread, PathList::default(), cx)
        });
        save_first.await.unwrap();
        let save_second = thread_store.update(cx, |store, cx| {
            store.save_thread(second_id.clone(), second_thread, PathList::default(), cx)
        });
        save_second.await.unwrap();
        cx.run_until_parked();

        let updated_first = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 3, 0, 0, 0).unwrap(),
        );
        let update_task = thread_store.update(cx, |store, cx| {
            store.save_thread(first_id.clone(), updated_first, PathList::default(), cx)
        });
        update_task.await.unwrap();
        cx.run_until_parked();

        let entries: Vec<_> = thread_store.read_with(cx, |store, _cx| store.entries().collect());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, first_id);
        assert_eq!(entries[1].id, second_id);
    }
}
