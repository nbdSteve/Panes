//! Integration test for Mem0Store against a running sidecar.
//!
//! Requires the sidecar to be running on port 11435:
//!   cd sidecar && source .venv/bin/activate && AWS_PROFILE=bedrock-beta python3 mem0_sidecar.py
//!
//! Run with: cargo test --test mem0_integration -p panes-memory -- --nocapture

use panes_memory::mem0_store::Mem0Store;
use panes_memory::MemoryStore;

const SIDECAR_URL: &str = "http://127.0.0.1:11435";

async fn sidecar_available() -> bool {
    let store = Mem0Store::new(SIDECAR_URL);
    store.health_check().await.unwrap_or(false)
}

#[tokio::test]
async fn test_mem0_health_check() {
    if !sidecar_available().await {
        eprintln!("SKIP: mem0 sidecar not running on {SIDECAR_URL}");
        return;
    }
    let store = Mem0Store::new(SIDECAR_URL);
    assert!(store.health_check().await.unwrap());
}

#[tokio::test]
async fn test_mem0_add_and_search() {
    if !sidecar_available().await {
        eprintln!("SKIP: mem0 sidecar not running on {SIDECAR_URL}");
        return;
    }

    let store = Mem0Store::new(SIDECAR_URL);
    let ws_id = format!("integration-test-{}", uuid::Uuid::new_v4());

    let transcript = "User: Always use Rust for backend services.\n\
                      Assistant: Understood, Rust for all backend work.";

    eprintln!("=== Adding transcript to mem0 (workspace_id={ws_id}) ===");
    let memories = store
        .add(transcript, Some(&ws_id), "integ-thread-1")
        .await
        .expect("add should succeed");

    eprintln!("Extracted {} memories:", memories.len());
    for m in &memories {
        eprintln!("  - [{}] {}", m.memory_type, m.content);
    }
    assert!(!memories.is_empty(), "should extract at least one memory");

    eprintln!("=== Searching ===");
    let results = store
        .search("what language for backend", Some(&ws_id), 5)
        .await
        .expect("search should succeed");

    eprintln!("Found {} results", results.len());
    for r in &results {
        eprintln!("  - {}", r.content);
    }
    assert!(!results.is_empty(), "search should find the Rust memory");

    // Cleanup
    let all = store.get_all(Some(&ws_id)).await.unwrap_or_default();
    for m in &all {
        let _ = store.delete(&m.id).await;
    }
}

#[tokio::test]
async fn test_mem0_dedup() {
    if !sidecar_available().await {
        eprintln!("SKIP: mem0 sidecar not running on {SIDECAR_URL}");
        return;
    }

    let store = Mem0Store::new(SIDECAR_URL);
    let ws_id = format!("dedup-test-{}", uuid::Uuid::new_v4());

    let t1 = "User: Use pnpm for package management.\nAssistant: Got it, pnpm only.";
    let memories1 = store.add(t1, Some(&ws_id), "t1").await.unwrap();
    let count1 = store.get_all(Some(&ws_id)).await.unwrap().len();
    eprintln!("After transcript 1: {} extracted, {} total", memories1.len(), count1);

    let t2 = "User: Remember, always pnpm.\nAssistant: Yes, pnpm.";
    let memories2 = store.add(t2, Some(&ws_id), "t2").await.unwrap();
    let count2 = store.get_all(Some(&ws_id)).await.unwrap().len();
    eprintln!("After transcript 2: {} operations, {} total", memories2.len(), count2);

    assert!(count2 <= count1 + 1, "dedup should prevent unbounded growth: had {count1}, now {count2}");

    // Cleanup
    let all = store.get_all(Some(&ws_id)).await.unwrap_or_default();
    for m in &all {
        let _ = store.delete(&m.id).await;
    }
}

#[tokio::test]
async fn test_mem0_get_all_and_delete() {
    if !sidecar_available().await {
        eprintln!("SKIP: mem0 sidecar not running on {SIDECAR_URL}");
        return;
    }

    let store = Mem0Store::new(SIDECAR_URL);
    let ws_id = format!("crud-test-{}", uuid::Uuid::new_v4());

    store.add("User: Use tabs.\nAssistant: Tabs it is.", Some(&ws_id), "t1").await.unwrap();

    let all = store.get_all(Some(&ws_id)).await.unwrap();
    assert!(!all.is_empty());

    let id = &all[0].id;
    store.delete(id).await.expect("delete should succeed");

    let after = store.get_all(Some(&ws_id)).await.unwrap();
    assert_eq!(after.len(), all.len() - 1);
}
