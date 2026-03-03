use layer0::effect::Scope;
use layer0::id::SessionId;
use layer0::state::{StateReader, StateStore};
use neuron_state_fs::FsStore;
use std::sync::Arc;

fn session_scope(id: &str) -> Scope {
    Scope::Session(SessionId::new(id))
}

// --- Basic CRUD ---

#[tokio::test]
async fn write_then_read() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    store
        .write(&scope, "key1", serde_json::json!("hello"))
        .await
        .unwrap();

    let val = StateStore::read(&store, &scope, "key1").await.unwrap();
    assert_eq!(val, Some(serde_json::json!("hello")));
}

#[tokio::test]
async fn read_missing_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    let val = StateStore::read(&store, &scope, "missing").await.unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn overwrite_replaces_value() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    store
        .write(&scope, "key1", serde_json::json!(1))
        .await
        .unwrap();
    store
        .write(&scope, "key1", serde_json::json!(2))
        .await
        .unwrap();

    let val = StateStore::read(&store, &scope, "key1").await.unwrap();
    assert_eq!(val, Some(serde_json::json!(2)));
}

#[tokio::test]
async fn delete_removes_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    store
        .write(&scope, "key1", serde_json::json!("val"))
        .await
        .unwrap();
    store.delete(&scope, "key1").await.unwrap();

    let val = StateStore::read(&store, &scope, "key1").await.unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn delete_missing_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    store.delete(&scope, "nonexistent").await.unwrap();
}

// --- List ---

#[tokio::test]
async fn list_by_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    store
        .write(&scope, "user:name", serde_json::json!("alice"))
        .await
        .unwrap();
    store
        .write(&scope, "user:email", serde_json::json!("a@b.com"))
        .await
        .unwrap();
    store
        .write(&scope, "config:theme", serde_json::json!("dark"))
        .await
        .unwrap();

    let mut keys = StateStore::list(&store, &scope, "user:").await.unwrap();
    keys.sort();
    assert_eq!(keys, vec!["user:email", "user:name"]);
}

#[tokio::test]
async fn list_empty_prefix_returns_all() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    store
        .write(&scope, "a", serde_json::json!(1))
        .await
        .unwrap();
    store
        .write(&scope, "b", serde_json::json!(2))
        .await
        .unwrap();

    let mut keys = StateStore::list(&store, &scope, "").await.unwrap();
    keys.sort();
    assert_eq!(keys, vec!["a", "b"]);
}

// --- Scope isolation ---

#[tokio::test]
async fn scopes_are_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let s1 = session_scope("s1");
    let s2 = session_scope("s2");

    store
        .write(&s1, "key", serde_json::json!("from-s1"))
        .await
        .unwrap();
    store
        .write(&s2, "key", serde_json::json!("from-s2"))
        .await
        .unwrap();

    assert_eq!(
        StateStore::read(&store, &s1, "key").await.unwrap(),
        Some(serde_json::json!("from-s1"))
    );
    assert_eq!(
        StateStore::read(&store, &s2, "key").await.unwrap(),
        Some(serde_json::json!("from-s2"))
    );
}

// --- Search ---

#[tokio::test]
async fn search_returns_empty_vec() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    store
        .write(&scope, "key1", serde_json::json!("hello"))
        .await
        .unwrap();

    let results = StateStore::search(&store, &scope, "hello", 10)
        .await
        .unwrap();
    assert!(results.is_empty());
}

// --- Object safety ---

#[tokio::test]
async fn usable_as_dyn_state_store() {
    let dir = tempfile::tempdir().unwrap();
    let store: Box<dyn StateStore> = Box::new(FsStore::new(dir.path()));
    let scope = session_scope("s1");

    store
        .write(&scope, "key", serde_json::json!("val"))
        .await
        .unwrap();
    let val = store.read(&scope, "key").await.unwrap();
    assert_eq!(val, Some(serde_json::json!("val")));
}

#[tokio::test]
async fn usable_as_arc_dyn_state_store() {
    let dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn StateStore> = Arc::new(FsStore::new(dir.path()));
    let scope = session_scope("s1");

    store
        .write(&scope, "key", serde_json::json!("val"))
        .await
        .unwrap();
    let val = store.read(&scope, "key").await.unwrap();
    assert_eq!(val, Some(serde_json::json!("val")));
}

// --- StateReader view ---

#[tokio::test]
async fn usable_as_dyn_state_reader() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");
    store
        .write(&scope, "key", serde_json::json!("val"))
        .await
        .unwrap();

    let reader: &dyn StateReader = &store;
    let val = reader.read(&scope, "key").await.unwrap();
    assert_eq!(val, Some(serde_json::json!("val")));
}

// --- Persistence ---

#[tokio::test]
async fn data_persists_across_store_instances() {
    let dir = tempfile::tempdir().unwrap();
    let scope = session_scope("s1");

    {
        let store = FsStore::new(dir.path());
        store
            .write(&scope, "persistent", serde_json::json!("survives"))
            .await
            .unwrap();
    }

    // New store instance, same directory
    let store = FsStore::new(dir.path());
    let val = StateStore::read(&store, &scope, "persistent")
        .await
        .unwrap();
    assert_eq!(val, Some(serde_json::json!("survives")));
}

// --- Complex values ---

#[tokio::test]
async fn stores_complex_json_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStore::new(dir.path());
    let scope = session_scope("s1");

    let complex = serde_json::json!({
        "messages": [
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "hi there"}
        ],
        "metadata": {"turn_count": 5}
    });

    store
        .write(&scope, "conversation", complex.clone())
        .await
        .unwrap();
    let val = StateStore::read(&store, &scope, "conversation")
        .await
        .unwrap();
    assert_eq!(val, Some(complex));
}
