//! Integration / e2e tests for Mem0Store against a running sidecar.
//!
//! Requires the sidecar to be running on port 11435:
//!   cd sidecar && source .venv/bin/activate && AWS_PROFILE=bedrock-beta python3 mem0_sidecar.py
//!
//! Run with: cargo test --test mem0_integration -p panes-memory -- --nocapture

use panes_memory::mem0_store::Mem0Store;
use panes_memory::sqlite_store::SqliteMemoryStore;
use panes_memory::{build_context, BriefingStore, MemoryStore};

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

// --- e2e tests: full injection pipeline, pin round-trip, workspace isolation, update ---

#[tokio::test]
async fn test_e2e_build_context_with_mem0_and_briefing() {
    if !sidecar_available().await {
        eprintln!("SKIP: mem0 sidecar not running on {SIDECAR_URL}");
        return;
    }

    let mem0 = Mem0Store::new(SIDECAR_URL);
    let briefing_store = SqliteMemoryStore::new(":memory:").unwrap();
    let ws_id = format!("e2e-ctx-{}", uuid::Uuid::new_v4());

    briefing_store.set_briefing(&ws_id, "Always use TypeScript strict mode").await.unwrap();

    let transcript = "User: We use pnpm, never npm.\nAssistant: Noted, pnpm only.";
    mem0.add(transcript, Some(&ws_id), "t1").await.unwrap();

    let ctx = build_context(&mem0, &briefing_store, "package manager", &ws_id, 2000)
        .await
        .unwrap();

    eprintln!("build_context returned: briefing={:?}, {} memories, ~{} tokens",
        ctx.briefing.is_some(), ctx.memories.len(), ctx.token_estimate);
    for m in &ctx.memories {
        eprintln!("  - [{}] {}", m.memory_type, m.content);
    }

    assert_eq!(ctx.briefing.as_deref(), Some("Always use TypeScript strict mode"));
    assert!(!ctx.memories.is_empty(), "should inject mem0 memories into context");

    // Cleanup
    let all = mem0.get_all(Some(&ws_id)).await.unwrap_or_default();
    for m in &all { let _ = mem0.delete(&m.id).await; }
}

#[tokio::test]
async fn test_e2e_pin_round_trip() {
    if !sidecar_available().await {
        eprintln!("SKIP: mem0 sidecar not running on {SIDECAR_URL}");
        return;
    }

    let store = Mem0Store::new(SIDECAR_URL);
    let ws_id = format!("e2e-pin-{}", uuid::Uuid::new_v4());

    store.add("User: Always use dark mode.\nAssistant: Dark mode it is.", Some(&ws_id), "t1")
        .await.unwrap();

    let all = store.get_all(Some(&ws_id)).await.unwrap();
    assert!(!all.is_empty());
    let mem_id = &all[0].id;

    assert!(!all[0].pinned, "should start unpinned");

    store.pin(mem_id, true).await.unwrap();

    let after_pin = store.get_all(Some(&ws_id)).await.unwrap();
    let pinned_mem = after_pin.iter().find(|m| &m.id == mem_id).unwrap();
    assert!(pinned_mem.pinned, "should be pinned after pin()");

    let search_results = store.search("dark mode", Some(&ws_id), 5).await.unwrap();
    if let Some(found) = search_results.iter().find(|m| &m.id == mem_id) {
        assert!(found.pinned, "search should also reflect pin state");
    }

    store.pin(mem_id, false).await.unwrap();
    let after_unpin = store.get_all(Some(&ws_id)).await.unwrap();
    let unpinned_mem = after_unpin.iter().find(|m| &m.id == mem_id).unwrap();
    assert!(!unpinned_mem.pinned, "should be unpinned after unpin");

    // Cleanup
    for m in &after_unpin { let _ = store.delete(&m.id).await; }
}

#[tokio::test]
async fn test_e2e_workspace_isolation() {
    if !sidecar_available().await {
        eprintln!("SKIP: mem0 sidecar not running on {SIDECAR_URL}");
        return;
    }

    let store = Mem0Store::new(SIDECAR_URL);
    let ws_a = format!("e2e-iso-a-{}", uuid::Uuid::new_v4());
    let ws_b = format!("e2e-iso-b-{}", uuid::Uuid::new_v4());

    let a_added = store.add(
        "User: Always use Python for data pipelines.\nAssistant: Understood, Python for all data work.",
        Some(&ws_a), "t1",
    ).await.unwrap();
    let b_added = store.add(
        "User: Always use Go for microservices.\nAssistant: Got it, Go for all services.",
        Some(&ws_b), "t1",
    ).await.unwrap();

    eprintln!("ws_a extracted {} memories, ws_b extracted {}", a_added.len(), b_added.len());
    assert!(!a_added.is_empty(), "ws_a add should extract memories");
    assert!(!b_added.is_empty(), "ws_b add should extract memories");

    let a_search = store.search("programming language", Some(&ws_a), 5).await.unwrap();
    let b_search = store.search("programming language", Some(&ws_b), 5).await.unwrap();

    eprintln!("ws_a search: {:?}", a_search.iter().map(|m| &m.content).collect::<Vec<_>>());
    eprintln!("ws_b search: {:?}", b_search.iter().map(|m| &m.content).collect::<Vec<_>>());

    for m in &a_search {
        assert!(!m.content.to_lowercase().contains(" go ") || !m.content.to_lowercase().contains("microservice"),
            "ws_a search leaked ws_b content: {}", m.content);
    }
    for m in &b_search {
        assert!(!m.content.to_lowercase().contains("python") || !m.content.to_lowercase().contains("data pipeline"),
            "ws_b search leaked ws_a content: {}", m.content);
    }

    // Cleanup
    for m in &store.get_all(Some(&ws_a)).await.unwrap_or_default() { let _ = store.delete(&m.id).await; }
    for m in &store.get_all(Some(&ws_b)).await.unwrap_or_default() { let _ = store.delete(&m.id).await; }
}

#[tokio::test]
async fn test_e2e_update() {
    if !sidecar_available().await {
        eprintln!("SKIP: mem0 sidecar not running on {SIDECAR_URL}");
        return;
    }

    let store = Mem0Store::new(SIDECAR_URL);
    let ws_id = format!("e2e-upd-{}", uuid::Uuid::new_v4());

    let added = store.add(
        "User: Use spaces for indentation.\nAssistant: Spaces it is.",
        Some(&ws_id), "t1",
    ).await.unwrap();
    assert!(!added.is_empty(), "add should extract at least one memory");
    let mem_id = &added[0].id;

    eprintln!("Updating memory {mem_id}: '{}' -> 'User prefers tabs over spaces'", added[0].content);
    store.update(mem_id, "User prefers tabs over spaces").await.unwrap();

    let after = store.get_all(Some(&ws_id)).await.unwrap();
    let updated = after.iter().find(|m| &m.id == mem_id)
        .expect("updated memory should still exist in get_all");
    assert_eq!(updated.content, "User prefers tabs over spaces");

    // Cleanup
    for m in &after { let _ = store.delete(&m.id).await; }
}
