//! "One writer, one announcement" (#629 / R1): every shared writer that
//! mutates durable state announces it on the daemon bus, so the agent's path
//! and the human's HTTP path are indistinguishable from a second open client.
//!
//! Each test here fails on the pre-R1 code, and the reason is stated on the
//! test. They are integration tests rather than unit tests because they must
//! exercise the PUBLIC writers — `Config::set_param`, `cards::create_card`,
//! `projects::create_project`, `finance_ledger::add_watchlist` — which is the
//! only way to prove the emit is on the writer and not on some caller.
//!
//! The event bus is a process-global singleton, so `subscribe()` before the
//! write and drain after; frames from other tests in the same binary are
//! filtered out by the ids/keys each test uses.

use permagent::events::{self, PermagentEvent, PermagentEventType};
use sqlx::{Pool, Sqlite};
use tokio::sync::broadcast::Receiver;

// ── helpers ─────────────────────────────────────────────────────────────────

async fn test_pool() -> Pool<Sqlite> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    permagent::session::spectral_schema::init_spectral_db(&pool)
        .await
        .unwrap();
    pool
}

/// Drain everything the bus has queued for this receiver.
fn drain(rx: &mut Receiver<PermagentEvent>) -> Vec<PermagentEvent> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

fn of_type(frames: &[PermagentEvent], t: PermagentEventType) -> Vec<&PermagentEvent> {
    frames.iter().filter(|e| e.event_type == t).collect()
}

fn str_field<'a>(e: &'a PermagentEvent, key: &str) -> Option<&'a str> {
    e.payload.get(key).and_then(|v| v.as_str())
}

/// Does any `config_changed` frame name `key`?
fn names_key(frames: &[PermagentEvent], key: &str) -> bool {
    of_type(frames, PermagentEventType::ConfigChanged)
        .iter()
        .any(|e| {
            e.payload
                .get("keys")
                .and_then(|v| v.as_array())
                .is_some_and(|a| a.iter().any(|k| k.as_str() == Some(key)))
        })
}

/// A project of this test's own, with default board columns seeded.
async fn own_project(pool: &Pool<Sqlite>, name: &str) -> String {
    permagent::projects::create_project(
        pool,
        permagent::projects::CreateProject {
            name: name.to_string(),
            slug: None,
            description: None,
            root_path: None,
            site_url: None,
            repo_url: None,
            notes: None,
            tags: None,
        },
    )
    .await
    .unwrap()
    .id
}

// ── 1. config_changed ───────────────────────────────────────────────────────

/// FAILS BEFORE: `Config::set_param` emitted nothing at all — there was no
/// `config_changed` event type. A key changed by the agent, the CLI, or a
/// second device left every open Settings pane showing the old value with
/// nothing to correct it.
#[test]
fn a_config_write_announces_the_key_it_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let config = permagent::config::Config::new_with_file_secrets(
        tmp.path().join("config.yaml"),
        tmp.path().join("secrets.yaml"),
    )
    .unwrap();

    let mut rx = events::subscribe();
    config.set_param("R1_TEST_KEY", "gpt-5").unwrap();
    let frames = drain(&mut rx);

    assert!(
        names_key(&frames, "R1_TEST_KEY"),
        "set_param did not emit config_changed naming the key; got {:?}",
        frames.iter().map(|f| &f.event_type).collect::<Vec<_>>()
    );
    let f = of_type(&frames, PermagentEventType::ConfigChanged)
        .into_iter()
        .find(|e| {
            e.payload["keys"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k.as_str() == Some("R1_TEST_KEY"))
        })
        .unwrap();
    assert_eq!(str_field(f, "change"), Some("set"));
    assert_eq!(f.payload["secret"], serde_json::Value::Bool(false));
}

/// FAILS BEFORE: no event existed, so there was nothing to check the payload
/// of. This is the security half of the frame: a secret write must announce the
/// KEY NAME and nothing about the value — not the value, not a prefix, not a
/// length. The bus is replayed to every client from a 1000-frame buffer, so a
/// value put here outlives the request that set it.
#[test]
fn a_secret_write_announces_the_name_and_never_the_value() {
    let tmp = tempfile::tempdir().unwrap();
    let config = permagent::config::Config::new_with_file_secrets(
        tmp.path().join("config.yaml"),
        tmp.path().join("secrets.yaml"),
    )
    .unwrap();

    const SECRET: &str = "sk-r1-do-not-leak-me-0123456789";
    let mut rx = events::subscribe();
    config.set_secret("R1_TEST_API_KEY", &SECRET).unwrap();
    let frames = drain(&mut rx);

    let f = of_type(&frames, PermagentEventType::ConfigChanged)
        .into_iter()
        .find(|e| {
            e.payload["keys"]
                .as_array()
                .is_some_and(|a| a.iter().any(|k| k.as_str() == Some("R1_TEST_API_KEY")))
        })
        .expect("set_secret did not emit config_changed");

    assert_eq!(
        f.payload["secret"],
        serde_json::Value::Bool(true),
        "a secret write must be flagged so clients never render the frame as a value"
    );
    let serialized = serde_json::to_string(&f).unwrap();
    assert!(
        !serialized.contains(SECRET),
        "SECRET LEAKED ONTO THE BUS: {serialized}"
    );
    // Not even a fragment: a prefix is still a credential disclosure.
    assert!(
        !serialized.contains("sk-r1"),
        "secret fragment on the bus: {serialized}"
    );
}

/// FAILS BEFORE: no event existed. Also pins the "REAL mutations only" rule
/// this bus is documented on — re-writing a key at its current value is a file
/// write but not a change, and must not churn an open Settings pane.
#[test]
fn rewriting_a_key_at_its_current_value_announces_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = permagent::config::Config::new_with_file_secrets(
        tmp.path().join("config.yaml"),
        tmp.path().join("secrets.yaml"),
    )
    .unwrap();
    config.set_param("R1_IDEMPOTENT", "same").unwrap();

    let mut rx = events::subscribe();
    config.set_param("R1_IDEMPOTENT", "same").unwrap();
    let frames = drain(&mut rx);

    assert!(
        !names_key(&frames, "R1_IDEMPOTENT"),
        "an unchanged rewrite announced a change"
    );

    // …but a real change still does.
    let mut rx = events::subscribe();
    config.set_param("R1_IDEMPOTENT", "different").unwrap();
    assert!(
        names_key(&drain(&mut rx), "R1_IDEMPOTENT"),
        "a real change stopped announcing"
    );
}

/// FAILS BEFORE: no event existed. Deleting a key is as much a Settings change
/// as setting one (it reverts the pane to the daemon default).
#[test]
fn deleting_a_key_announces_it_and_a_no_op_delete_does_not() {
    let tmp = tempfile::tempdir().unwrap();
    let config = permagent::config::Config::new_with_file_secrets(
        tmp.path().join("config.yaml"),
        tmp.path().join("secrets.yaml"),
    )
    .unwrap();
    config.set_param("R1_DELETE_ME", "x").unwrap();

    let mut rx = events::subscribe();
    config.delete("R1_DELETE_ME").unwrap();
    let frames = drain(&mut rx);
    let f = of_type(&frames, PermagentEventType::ConfigChanged)
        .into_iter()
        .find(|e| {
            e.payload["keys"]
                .as_array()
                .is_some_and(|a| a.iter().any(|k| k.as_str() == Some("R1_DELETE_ME")))
        })
        .expect("delete did not emit config_changed");
    assert_eq!(str_field(f, "change"), Some("deleted"));

    let mut rx = events::subscribe();
    config.delete("R1_DELETE_ME").unwrap();
    assert!(
        !names_key(&drain(&mut rx), "R1_DELETE_ME"),
        "deleting an absent key announced a change"
    );
}

// ── 3. cards / columns ──────────────────────────────────────────────────────

/// FAILS BEFORE: NEITHER path emitted. `cards::create_card` was silent and so
/// was `POST /api/projects/{id}/cards`, so a card added by the agent — or on
/// another device — never appeared on an open Kanban board. The board listens
/// to `project_changed`, which is why that is the frame rather than a new
/// `card_changed` nobody consumes.
#[tokio::test]
async fn every_card_write_announces_the_board_it_changed() {
    let pool = test_pool().await;
    // Its OWN project: tests in one binary share the process-global bus, so a
    // sibling test writing cards on the Personal board would be counted here.
    let project = own_project(&pool, "R1 Card Probe").await;
    let project_id = project.as_str();

    let cards_frames = |frames: &[PermagentEvent]| -> usize {
        of_type(frames, PermagentEventType::ProjectChanged)
            .iter()
            .filter(|e| {
                str_field(e, "change") == Some("cards")
                    && str_field(e, "project_id") == Some(project_id)
            })
            .count()
    };

    // create
    let mut rx = events::subscribe();
    let card = permagent::cards::create_card(
        &pool,
        permagent::cards::CreateCard {
            project_id: project_id.to_string(),
            title: "R1 probe".to_string(),
            description: None,
            card_type: None,
            column_id: None,
            created_by: None,
            metadata_json: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(cards_frames(&drain(&mut rx)), 1, "create_card was silent");

    // update
    let mut rx = events::subscribe();
    permagent::cards::update_card(
        &pool,
        &card.id,
        permagent::cards::UpdateCard {
            title: Some("R1 probe renamed".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(cards_frames(&drain(&mut rx)), 1, "update_card was silent");

    // move
    let cols = permagent::cards::list_columns(&pool, project_id)
        .await
        .unwrap();
    let target = cols
        .iter()
        .find(|c| c.id != card.column_id)
        .expect("board has more than one column");
    let mut rx = events::subscribe();
    permagent::cards::move_card(&pool, &card.id, &target.id, None)
        .await
        .unwrap();
    assert_eq!(cards_frames(&drain(&mut rx)), 1, "move_card was silent");

    // column create
    let mut rx = events::subscribe();
    let col = permagent::cards::create_column(
        &pool,
        permagent::cards::CreateColumn {
            project_id: project_id.to_string(),
            name: "R1 column".to_string(),
            position: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(cards_frames(&drain(&mut rx)), 1, "create_column was silent");

    // column delete (empty, so it is allowed)
    let mut rx = events::subscribe();
    assert!(permagent::cards::delete_column(&pool, &col.id)
        .await
        .unwrap());
    assert_eq!(cards_frames(&drain(&mut rx)), 1, "delete_column was silent");

    // card delete
    let mut rx = events::subscribe();
    assert!(permagent::cards::delete_card(&pool, &card.id)
        .await
        .unwrap());
    assert_eq!(cards_frames(&drain(&mut rx)), 1, "delete_card was silent");
}

/// FAILS BEFORE: nothing emitted, so there was no over-emit to guard against
/// either. Guards the shape of the fix: a batch reorder must be ONE frame per
/// board, not one per card — the listener refetches the whole board regardless,
/// and a 200-card drag would otherwise burn a fifth of the replay buffer.
#[tokio::test]
async fn a_batch_reorder_announces_once_per_board_not_once_per_card() {
    let pool = test_pool().await;
    let project = own_project(&pool, "R1 Reorder Probe").await;
    let project_id = project.as_str();
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            permagent::cards::create_card(
                &pool,
                permagent::cards::CreateCard {
                    project_id: project_id.to_string(),
                    title: format!("R1 reorder {i}"),
                    description: None,
                    card_type: None,
                    column_id: None,
                    created_by: None,
                    metadata_json: None,
                },
            )
            .await
            .unwrap(),
        );
    }
    let moves: Vec<(String, String, i32)> = ids
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.clone(), c.column_id.clone(), (4 - i) as i32))
        .collect();

    let mut rx = events::subscribe();
    permagent::cards::reorder_cards(&pool, &moves)
        .await
        .unwrap();
    let n = of_type(&drain(&mut rx), PermagentEventType::ProjectChanged)
        .iter()
        .filter(|e| {
            str_field(e, "change") == Some("cards")
                && str_field(e, "project_id") == Some(project_id)
        })
        .count();
    assert_eq!(n, 1, "a 5-card reorder emitted {n} frames; expected 1");
}

// ── 4. projects ─────────────────────────────────────────────────────────────

/// FAILS BEFORE: the emit lived on the HTTP handlers, so
/// `platform__project(create)` and `platform__project(delete)` — which call
/// these same functions — announced nothing. Moving it onto the writer is the
/// #1090 pattern; the route duplicates are deleted in the same change, which is
/// what the exact frame counts here pin.
#[tokio::test]
async fn project_create_and_delete_announce_from_the_writer() {
    let pool = test_pool().await;

    let mut rx = events::subscribe();
    let project = permagent::projects::create_project(
        &pool,
        permagent::projects::CreateProject {
            name: "R1 Writer Probe".to_string(),
            slug: None,
            description: None,
            root_path: None,
            site_url: None,
            repo_url: None,
            notes: None,
            tags: None,
        },
    )
    .await
    .unwrap();
    let created: Vec<_> = of_type(&drain(&mut rx), PermagentEventType::ProjectChanged)
        .iter()
        .filter(|e| str_field(e, "project_id") == Some(project.id.as_str()))
        .map(|e| str_field(e, "change").unwrap_or("").to_string())
        .collect();
    assert!(
        created.contains(&"created".to_string()),
        "create_project was silent; saw {created:?}"
    );
    assert_eq!(
        created.iter().filter(|c| *c == "created").count(),
        1,
        "double announcement — a route emit was left behind"
    );

    let mut rx = events::subscribe();
    assert!(permagent::projects::delete_project(&pool, &project.id)
        .await
        .unwrap());
    let deleted = of_type(&drain(&mut rx), PermagentEventType::ProjectChanged)
        .iter()
        .filter(|e| {
            str_field(e, "project_id") == Some(project.id.as_str())
                && str_field(e, "change") == Some("deleted")
        })
        .count();
    assert_eq!(deleted, 1, "delete_project announced {deleted} times");
}

// ── 5. finance ──────────────────────────────────────────────────────────────

/// FAILS BEFORE: no `finance_changed` frame existed and neither the routes nor
/// the 24 `platform__finance` tools emitted anything. The Finance view's only
/// way to notice a change — including one the agent made in the conversation
/// the user was watching — was its 60-second poll.
#[tokio::test]
async fn finance_ledger_writes_announce_their_kind() {
    let pool = test_pool().await;

    let saw = |frames: &[PermagentEvent], kind: &str, change: &str| -> bool {
        of_type(frames, PermagentEventType::FinanceChanged)
            .iter()
            .any(|e| str_field(e, "kind") == Some(kind) && str_field(e, "change") == Some(change))
    };

    let mut rx = events::subscribe();
    permagent::finance_ledger::add_watchlist(&pool, "r1tst", None, None)
        .await
        .unwrap();
    assert!(
        saw(&drain(&mut rx), "watchlist", "created"),
        "add_watchlist was silent"
    );

    let mut rx = events::subscribe();
    let note = permagent::finance_ledger::add_note(&pool, "R1", "probe", None)
        .await
        .unwrap();
    assert!(
        saw(&drain(&mut rx), "note", "created"),
        "add_note was silent"
    );

    let mut rx = events::subscribe();
    assert!(permagent::finance_ledger::delete_note(&pool, &note.id)
        .await
        .unwrap());
    assert!(
        saw(&drain(&mut rx), "note", "deleted"),
        "delete_note was silent"
    );

    let mut rx = events::subscribe();
    let pos = permagent::finance_ledger::add_position(
        &pool,
        permagent::finance_ledger::NewPosition {
            symbol: "R1TST".to_string(),
            company_name: String::new(),
            entry_date: "2026-09-01".to_string(),
            entry_price: 10.0,
            shares: 1,
            exit_date: None,
            exit_price: None,
            notes: None,
        },
    )
    .await
    .unwrap();
    assert!(
        saw(&drain(&mut rx), "position", "created"),
        "add_position was silent"
    );

    // A failed write announces nothing — the bus is for real mutations.
    let mut rx = events::subscribe();
    assert!(
        permagent::finance_ledger::delete_position(&pool, "no-such-position")
            .await
            .is_ok_and(|deleted| !deleted)
    );
    assert!(
        !saw(&drain(&mut rx), "position", "deleted"),
        "a delete that removed nothing announced a change"
    );

    let mut rx = events::subscribe();
    assert!(permagent::finance_ledger::delete_position(&pool, &pos.id)
        .await
        .unwrap());
    assert!(
        saw(&drain(&mut rx), "position", "deleted"),
        "delete_position was silent"
    );
}

/// FAILS BEFORE: no frame existed. Payload discipline for a domain that is
/// literally money: the frame says a kind changed, never how much.
#[test]
fn a_finance_frame_carries_no_figures() {
    let e = events::finance_changed("position", "created");
    let v = serde_json::to_value(&e).unwrap();
    let mut keys: Vec<&str> = v["payload"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["change", "kind"],
        "finance_changed grew a field; nothing about a position's value may ride this bus"
    );
}
