use super::*;

fn batch(source_count: usize, function_count: usize) -> CallBatch {
    CallBatch {
        lines: (0..function_count)
            .map(|index| (format!("test:{index}"), Arc::from([])))
            .collect::<Vec<_>>()
            .into(),
        sources: (0..source_count)
            .map(|_| {
                Some(FunctionSource {
                    source: Arc::new(CommandSource::dummy()),
                    result: FunctionResult::new(function_count, ResultValueTaker::new()),
                    feedback_source: None,
                })
            })
            .collect(),
        source_index: 0,
        function_index: 0,
    }
}

#[test]
fn queue_regression_cap_counts_calls_and_materialized_frames() -> Result<(), FunctionRunError> {
    let mut queue = FunctionQueue::new(100, 100, 5);
    // Vanilla checks > before enqueue: max + 1 entries fit, and the next call overflows.
    queue.enqueue(batch(2, 3))?;
    assert_eq!(queue.queued_calls, 6);
    assert!(matches!(queue.next(), Some(Work::Discarded)));
    assert_eq!(queue.queued_calls, 5);
    assert_eq!(queue.frames.len(), 1);
    assert!(matches!(queue.next(), Some(Work::Completed(_))));
    assert!(queue.frames.is_empty());
    queue.enqueue(batch(1, 1))?;
    assert_eq!(queue.queued_calls, 6);
    assert_eq!(
        queue.enqueue(batch(1, 1)),
        Err(FunctionRunError::ChainLimitExceeded)
    );
    assert!(queue.queue_overflow);
    assert_eq!(queue.queued_calls, 0);
    assert!(queue.next().is_none());

    let mut queue = FunctionQueue::new(100, 100, 5);
    queue.enqueue(batch(2, 3))?;
    assert!(matches!(queue.next(), Some(Work::Discarded)));
    assert_eq!(
        queue.enqueue(batch(1, 1)),
        Err(FunctionRunError::ChainLimitExceeded)
    );
    assert!(queue.frames.is_empty());
    assert!(queue.entries.is_empty());
    assert!(queue.pending.is_empty());
    Ok(())
}

#[test]
fn queue_regression_sources_release_at_their_last_function()
-> Result<(), Box<dyn std::error::Error>> {
    let mut batch = batch(2, 2);
    let source = Arc::downgrade(&batch.sources[0].as_ref().ok_or("missing source")?.source);
    let first = batch.next_frame().ok_or("missing first frame")?;
    assert!(batch.sources[0].is_some());
    let second = batch.next_frame().ok_or("missing second frame")?;
    assert!(batch.sources[0].is_none());
    assert_eq!(batch.remaining_calls(), 2);
    drop(first);
    drop(second);
    assert!(source.upgrade().is_none());
    assert!(batch.next_frame().is_some());
    assert!(batch.next_frame().is_some());
    assert!(batch.sources.iter().all(Option::is_none));
    assert_eq!(batch.remaining_calls(), 0);
    Ok(())
}

#[test]
fn review3_top_level_return_discards_pending_return_callbacks() -> Result<(), FunctionRunError> {
    let mut queue = FunctionQueue::new(100, 100, MAX_QUEUE_DEPTH);
    queue.enqueue(batch(1, 1))?;
    assert!(matches!(queue.next(), Some(Work::Discarded)));
    queue.delivering_returns = true;
    ACTIVE_QUEUE.with(|active| *active.borrow_mut() = Some(queue));
    let _guard = QueueGuard;
    return_from_function(0, ReturnValue::Success(7));
    ACTIVE_QUEUE.with(|active| {
        assert_eq!(active.borrow().as_ref().unwrap().pending_returns.len(), 1);
    });
    return_from_function(TOP_FRAME_ID, ReturnValue::Success(9));
    ACTIVE_QUEUE.with(|active| {
        let mut active = active.borrow_mut();
        let queue = active.as_mut().unwrap();
        assert!(queue.pending_returns.is_empty());
        assert!(queue.frames.is_empty());
        assert!(queue.entries.is_empty());
        assert!(queue.pending.is_empty());
        assert!(queue.next().is_none());
    });
    Ok(())
}
