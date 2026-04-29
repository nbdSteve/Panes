use panes_memory::sqlite_store::SqliteMemoryStore;
use panes_memory::{build_context, BriefingStore, MemoryStore};

fn make_store() -> SqliteMemoryStore {
    SqliteMemoryStore::new(":memory:").unwrap()
}

#[tokio::test]
async fn test_full_thread_lifecycle() {
    let store = make_store();
    let ws = "ws-lifecycle";

    store.set_briefing(ws, "Always use TypeScript strict mode").await.unwrap();

    let ctx = build_context(&store, &store, "create a greeting function", ws, 2000)
        .await
        .unwrap();
    assert_eq!(ctx.briefing.as_deref(), Some("Always use TypeScript strict mode"));
    assert!(ctx.memories.is_empty());

    let transcript = "User: create a greeting function\nAssistant: I created greeting.ts with a greet() function using TypeScript strict mode.";
    let extracted = store.add(transcript, Some(ws), "thread-1").await.unwrap();
    assert_eq!(extracted.len(), 1);
    assert!(extracted[0].content.contains("greeting"));

    let ctx2 = build_context(&store, &store, "greeting function TypeScript", ws, 2000)
        .await
        .unwrap();
    assert_eq!(ctx2.briefing.as_deref(), Some("Always use TypeScript strict mode"));
    assert!(!ctx2.memories.is_empty(), "second build_context should find the extracted memory");
}

#[tokio::test]
async fn test_memory_crud_round_trip() {
    let store = make_store();
    let ws = "ws-crud";

    let mems = store.add("User asked about pnpm", Some(ws), "t1").await.unwrap();
    let id = &mems[0].id;

    let all = store.get_all(Some(ws)).await.unwrap();
    assert_eq!(all.len(), 1);

    store.update(id, "Prefer pnpm over npm").await.unwrap();
    let all = store.get_all(Some(ws)).await.unwrap();
    assert_eq!(all[0].content, "Prefer pnpm over npm");
    assert!(all[0].edited_at.is_some());

    store.pin(id, true).await.unwrap();
    let all = store.get_all(Some(ws)).await.unwrap();
    assert!(all[0].pinned);

    store.pin(id, false).await.unwrap();
    let all = store.get_all(Some(ws)).await.unwrap();
    assert!(!all[0].pinned);

    store.delete(id).await.unwrap();
    let all = store.get_all(Some(ws)).await.unwrap();
    assert!(all.is_empty());
}

#[tokio::test]
async fn test_briefing_crud_round_trip() {
    let store = make_store();
    let ws = "ws-briefing";

    assert!(store.get_briefing(ws).await.unwrap().is_none());

    store.set_briefing(ws, "Use React 19").await.unwrap();
    let b = store.get_briefing(ws).await.unwrap().unwrap();
    assert_eq!(b.content, "Use React 19");

    store.set_briefing(ws, "Use React 19 with TypeScript").await.unwrap();
    let b = store.get_briefing(ws).await.unwrap().unwrap();
    assert_eq!(b.content, "Use React 19 with TypeScript");

    store.delete_briefing(ws).await.unwrap();
    assert!(store.get_briefing(ws).await.unwrap().is_none());
}

#[tokio::test]
async fn test_build_context_without_briefing() {
    let store = make_store();
    let ws = "ws-no-briefing";

    store.add("memory about testing", Some(ws), "t1").await.unwrap();

    let ctx = build_context(&store, &store, "testing memory", ws, 2000)
        .await
        .unwrap();

    assert!(ctx.briefing.is_none());
    assert!(!ctx.memories.is_empty());
}

#[tokio::test]
async fn test_build_context_pinned_always_included_in_injection() {
    let store = make_store();
    let ws = "ws-pin-inject";

    let mems = store.add("critical TypeScript constraint rule", Some(ws), "t1").await.unwrap();
    store.pin(&mems[0].id, true).await.unwrap();

    let ctx = build_context(&store, &store, "TypeScript constraint", ws, 2000)
        .await
        .unwrap();

    assert!(ctx.memories.iter().any(|m| m.pinned), "pinned memory should be in context");
}

#[tokio::test]
async fn test_workspace_isolation_in_context() {
    let store = make_store();

    store.add("ws1 memory about rust", Some("ws1"), "t1").await.unwrap();
    store.add("ws2 memory about python", Some("ws2"), "t2").await.unwrap();

    let ctx1 = build_context(&store, &store, "rust memory", "ws1", 2000).await.unwrap();
    let ctx2 = build_context(&store, &store, "python memory", "ws2", 2000).await.unwrap();

    for m in &ctx1.memories {
        assert!(
            m.workspace_id.as_deref() == Some("ws1") || m.workspace_id.is_none(),
            "ws1 context should not include ws2 memories"
        );
    }
    for m in &ctx2.memories {
        assert!(
            m.workspace_id.as_deref() == Some("ws2") || m.workspace_id.is_none(),
            "ws2 context should not include ws1 memories"
        );
    }
}

#[tokio::test]
async fn test_multiple_extractions_accumulate() {
    let store = make_store();
    let ws = "ws-accumulate";

    store.add("first thread: setup project", Some(ws), "t1").await.unwrap();
    store.add("second thread: add tests", Some(ws), "t2").await.unwrap();
    store.add("third thread: deploy app", Some(ws), "t3").await.unwrap();

    let all = store.get_all(Some(ws)).await.unwrap();
    assert_eq!(all.len(), 3);
}
