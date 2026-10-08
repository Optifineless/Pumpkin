use super::StagedChunkEnum;
use super::{ChunkPos, Duration, EdgeKey, GenerationSchedule, HashSetType, NodeKey, error};
use slotmap::Key;
use std::time::Instant;

impl GenerationSchedule {
    pub(super) fn finish_storage(&mut self) {
        // IOWorker.synchronize: every in-flight producer must return its chunks before
        // the final save. A timeout must not discard chunks still owned by generation.
        while self.running_task_count > 0 {
            match self.recv_chunk.recv_timeout(Duration::from_secs(5)) {
                Ok((pos, data)) => self.receive_chunk(pos, data),
                Err(error) => error!(
                    "Waiting for {} chunk tasks during shutdown: {error}",
                    self.running_task_count
                ),
            }
        }
        self.save_all_chunk(true);
    }

    // IOWorker.loadAsync completes exceptionally. Keep generation dependent on the
    // failed read, but complete waiters and retry I/O instead of abandoning an occupied node.
    pub(super) fn fail_storage_read(&mut self, pos: ChunkPos, error: &str) {
        self.failed_loads.insert(pos, Instant::now());
        self.listener.process_failed_chunk(pos, error);
        let Some(holder) = self.chunk_map.get_mut(&pos) else {
            return;
        };
        let occupied = holder.occupied;
        let task = std::mem::replace(
            &mut holder.tasks[StagedChunkEnum::Empty as usize],
            NodeKey::null(),
        );
        if let Some(node) = self.graph.nodes.get_mut(occupied) {
            node.in_flight = false;
        }
        for key in self.storage_blocked_nodes(occupied) {
            if let Some(node) = self.graph.nodes.get(key) {
                // IOWorker.loadAsync fails the requested read, not its healthy neighbors.
                self.listener.fail_current_listeners(node.pos, error);
            }
        }
        // Dependencies registered before dispatch may only point to the Empty task.
        // Transfer them to the failure barrier before releasing that in-flight node.
        let mut edge = self
            .graph
            .nodes
            .get(task)
            .map_or(EdgeKey::null(), |node| node.edge);
        while let Some((to, next)) = self.graph.edges.get(edge).map(|edge| (edge.to, edge.next)) {
            self.graph.add_edge(occupied, to);
            edge = next;
        }
        self.waiting_for_chunks.remove(&task);
        self.drop_node(task);
    }

    fn storage_blocked_nodes(&self, start: NodeKey) -> HashSetType<NodeKey> {
        let mut blocked = HashSetType::default();
        let mut queue = vec![start];
        while let Some(key) = queue.pop() {
            if !blocked.insert(key) {
                continue;
            }
            let Some(node) = self.graph.nodes.get(key) else {
                continue;
            };
            let mut edge = node.edge;
            while let Some(next) = self.graph.edges.get(edge) {
                queue.push(next.to);
                edge = next.next;
            }
        }
        blocked
    }

    pub(super) fn retry_storage_reads(&mut self) {
        // Fork recovery policy: retry unavailable storage once per second, never generate
        // from an error. Missing files on a later successful read follow the normal path.
        let retry: Vec<_> = self
            .failed_loads
            .iter()
            .filter(|(_, since)| since.elapsed() >= Duration::from_secs(1))
            .map(|(pos, _)| *pos)
            .collect();
        for pos in retry {
            if self.running_task_count >= self.max_in_flight {
                break;
            }
            if self.io_read.blocking_send(vec![pos]).is_err() {
                error!("Chunk read worker closed while retrying {pos:?}");
                break;
            }
            self.failed_loads.remove(&pos);
            self.running_task_count += 1;
            if let Some(holder) = self.chunk_map.get(&pos)
                && let Some(node) = self.graph.nodes.get_mut(holder.occupied)
            {
                node.in_flight = true;
            }
        }
    }

    pub(super) fn debug_check_storage_barriers(&self) -> bool {
        let mut blocked = HashSetType::default();
        for pos in self.failed_loads.keys() {
            let holder = &self.chunk_map[pos];
            debug_assert!(holder.chunk.is_none());
            blocked.extend(self.storage_blocked_nodes(holder.occupied));
        }
        // Every remaining task at idle must actually depend on a failed read.
        debug_assert!(self.graph.nodes.keys().all(|key| blocked.contains(&key)));
        true
    }
}
