use super::{CallBatch, Entry, Frame, FunctionQueue, FunctionRunError, Work};

impl FunctionQueue {
    pub(super) const fn new(limit: usize, fork_limit: usize, max_queue_depth: usize) -> Self {
        Self {
            frames: Vec::new(),
            entries: Vec::new(),
            pending: Vec::new(),
            pending_returns: std::collections::VecDeque::new(),
            delivering_returns: false,
            remaining: limit,
            next_id: 0,
            queued_calls: 0,
            max_queue_depth,
            queue_overflow: false,
            fork_limit,
            top_returned: false,
        }
    }

    pub(super) fn frame_mut(&mut self, id: usize) -> Option<&mut Frame> {
        let index = self
            .frames
            .binary_search_by_key(&id, |frame| frame.id)
            .ok()?;
        self.frames.get_mut(index)
    }

    pub(super) fn enqueue(&mut self, batch: CallBatch) -> Result<(), FunctionRunError> {
        let calls = batch.remaining_calls();
        // ExecutionContext.queueNext checks > before each CallFunction enqueue, admitting max + 1.
        // Batches count pending calls; frames count queued command continuations.
        let depth = self.frames.len().saturating_add(self.queued_calls);
        if self.queue_overflow
            || (calls != 0 && depth.saturating_add(calls - 1) > self.max_queue_depth)
        {
            // ExecutionContext.handleQueueOverflow discards all outstanding work.
            self.frames.clear();
            self.entries.clear();
            self.pending.clear();
            self.pending_returns.clear();
            self.queued_calls = 0;
            self.queue_overflow = true;
            return Err(FunctionRunError::ChainLimitExceeded);
        }
        self.queued_calls += calls;
        if calls != 0 {
            self.pending.push(Entry::Batch(batch));
        }
        Ok(())
    }

    // ExecutionContext.runCommandQueue and CallFunction: function calls and commands cost quota.
    pub(super) fn next(&mut self) -> Option<Work> {
        if self.queue_overflow || self.top_returned || self.remaining == 0 {
            return None;
        }
        self.entries.extend(self.pending.drain(..).rev());
        if let Some(Entry::Batch(_)) = self.entries.last() {
            if let Some(Entry::Batch(mut batch)) = self.entries.pop() {
                let before = batch.remaining_calls();
                let frame = batch.next_frame();
                let after = batch.remaining_calls();
                self.queued_calls -= before - after;
                if after != 0 {
                    self.entries.push(Entry::Batch(batch));
                }
                if let Some(mut frame) = frame {
                    frame.id = self.next_id;
                    self.next_id += 1;
                    self.entries.push(Entry::Frame);
                    self.frames.push(frame);
                }
            }
            return Some(Work::Discarded);
        }
        // CallFunction(returnParentFrame) discards every remaining child of the returned frame.
        let returning = self
            .frames
            .last()
            .and_then(|frame| frame.result.returning_frame());
        if returning.is_some_and(|id| self.frame_mut(id).is_some_and(|frame| frame.returned)) {
            self.frames.pop();
            self.entries.pop();
            return Some(Work::Discarded);
        }
        if let Some(frame) = self.frames.last_mut() {
            if frame.result.should_discard() {
                self.frames.pop();
                self.entries.pop();
                return Some(Work::Discarded);
            }
            if !frame.started {
                self.remaining -= 1;
                frame.started = true;
            }
            if let Some(line) = frame.lines.get(frame.index) {
                if self.remaining == 0 {
                    return None;
                }
                frame.index += 1;
                return Some(Work::Command(frame.source.clone(), line.clone()));
            }
            self.entries.pop();
            return self.frames.pop().map(|frame| Work::Completed(frame.result));
        }
        None
    }
}
