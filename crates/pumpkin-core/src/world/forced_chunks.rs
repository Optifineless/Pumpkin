//! ServerLevel.setChunkForced and entity visibility share the existing unload admission.
use std::sync::{Arc, PoisonError};

use pumpkin_util::{math::vector2::Vector2, sync::LockClass};

use super::World;

pub type ForceChange = tokio::task::JoinHandle<()>;
pub enum ForceChanges {}
impl LockClass for ForceChanges {
    const NAME: &'static str = "forced chunk task chain";
    const LEVEL: u8 = 20;
}
pub enum ForcedChunks {}
impl LockClass for ForcedChunks {
    const NAME: &'static str = "forced chunk membership";
    const LEVEL: u8 = 30;
}

impl World {
    /// Changes force tickets and authoritative watches synchronously, then schedules storage work.
    pub fn set_chunks_forced(self: &Arc<Self>, chunks: &[Vector2<i32>], forced: bool) {
        let server = self.server.upgrade();
        let token = server.as_ref().map(|server| server.tasks.token());
        let runtime = server.as_ref().map(|server| server.runtime.enter());
        let (mut previous_task, changed) = self.level.change_forced_entity_chunks(forced, || {
            let previous_task = self
                .force_change_task
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let changed = {
                let mut membership = self
                    .forced_chunks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                chunks
                    .iter()
                    .copied()
                    .filter(|chunk| {
                        if forced {
                            membership.insert(*chunk)
                        } else {
                            membership.remove(chunk)
                        }
                    })
                    .collect::<Vec<_>>()
            };
            {
                let mut loading = self
                    .level
                    .chunk_loading
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                for &chunk in &changed {
                    if forced {
                        loading.add_force_ticket(chunk);
                    } else {
                        loading.remove_force_ticket(chunk);
                    }
                }
            }
            (changed.clone(), (previous_task, changed))
        });
        if changed.is_empty() {
            return;
        }
        if server.is_some() {
            let previous = previous_task.take();
            let world = self.clone();
            // tokio::spawn never polls synchronously. Only runtime bookkeeping is under the existing
            // task-chain guard; admission has ended and the future owns no guard or object callback.
            *previous_task = Some(tokio::spawn(async move {
                let _token = token;
                if let Some(previous) = previous {
                    let _ = previous.await;
                }
                if forced {
                    world
                        .level
                        .update_entity_storage_watchers(&changed, &[])
                        .await;
                    world.spawn_forced_entity_chunks(changed);
                } else {
                    world
                        .level
                        .update_entity_storage_watchers(&[], &changed)
                        .await;
                    world.remove_entities_in_chunks(changed).await;
                }
            }));
        }
        drop(previous_task);
        drop(runtime);
        // Ticket notifications and active-chunk callbacks run after task-chain ownership ends.
        self.level
            .chunk_loading
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send_change();
        self.update_active_chunks();
    }
}
