//! The memory budget for results is shared by every open result.
//!
//! In a file of its own because the budget is process-wide: these checks
//! must not run alongside other tests that fill result stores.

use sqltool_core::store::{memory_in_use, set_memory_budget};
use sqltool_core::{ColumnInfo, ColumnKind, ResultStore, RowBatch};

fn fill(store: &ResultStore, rows: usize) {
    store.set_columns(vec![ColumnInfo::plain("text", ColumnKind::Text)]);
    let value = vec![b'x'; 1000];
    let mut batch = RowBatch::default();
    for _ in 0..rows {
        batch.push_row([Some(&value[..])]);
    }
    store.append(&mut batch);
    store.finish(None);
}

#[test]
fn open_results_share_one_memory_budget_and_give_it_back_when_closed() {
    set_memory_budget(1024 * 1024);
    assert_eq!(memory_in_use(), 0);

    // Each store alone is far under its own (large) allowance; 64 KB chunks.
    let own_allowance = 64 * 1024 * 1024;
    let first = ResultStore::with_limits(own_allowance, 64 * 1024);
    fill(&first, 800);
    assert_eq!(first.status().spilled_bytes, 0, "one result under the budget stays in memory");

    let second = ResultStore::with_limits(own_allowance, 64 * 1024);
    fill(&second, 800);
    assert!(second.status().spilled_bytes > 0, "the second result took the total past the budget, so it must use the disk");
    assert!(memory_in_use() < 1024 * 1024 + 2 * 64 * 1024, "in use: {}", memory_in_use());

    // What went to disk reads back the same.
    assert_eq!(second.cell(799, 0).unwrap().unwrap().len(), 1000);
    assert_eq!(second.status().rows, 800);

    drop(first);
    drop(second);
    assert_eq!(memory_in_use(), 0, "closing results returns their memory");
}
