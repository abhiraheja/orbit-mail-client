//! Dev utility: write key/value pairs into the running app's `app_settings`
//! table (e.g. OAuth client IDs) without going through the UI.
//!
//! Usage: cargo run --example set_setting -- <db_path> <key> <value> [<key> <value> ...]
//! Values are passed as args so secrets never live in source.

use rusqlite::{params, Connection};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: set_setting <db_path> [<key> <value> ...]");
        std::process::exit(2);
    }
    let conn = Connection::open(&args[1]).expect("open db");

    // With only the db path, dump settings.
    if args.len() == 2 {
        let mut stmt = conn.prepare("SELECT key, value FROM app_settings ORDER BY key").unwrap();
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap();
        for row in rows {
            let (k, v) = row.unwrap();
            println!("{k} = [{}] (len {})", v, v.len());
        }
        return;
    }

    // `diag`: dump sync state.
    if args.len() == 3 && args[2] == "diag" {
        let mut s = conn.prepare("SELECT id, email, provider, auth_kind, last_synced FROM accounts").unwrap();
        let rows = s.query_map([], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?, r.get::<_,String>(3)?, r.get::<_,Option<i64>>(4)?))).unwrap();
        for row in rows {
            let (id, email, prov, auth, last) = row.unwrap();
            println!("account #{id} {email} provider={prov} auth={auth} last_synced={last:?}");
        }
        let msgs: i64 = conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap();
        let threads: i64 = conn.query_row("SELECT count(*) FROM threads", [], |r| r.get(0)).unwrap();
        let loops: i64 = conn.query_row("SELECT count(*) FROM loops", [], |r| r.get(0)).unwrap();
        println!("messages={msgs} threads={threads} loops={loops}");
        return;
    }
    if (args.len() - 2) % 2 != 0 {
        eprintln!("usage: set_setting <db_path> [<key> <value> ...]");
        std::process::exit(2);
    }
    let mut i = 2;
    while i + 1 < args.len() {
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![&args[i], &args[i + 1]],
        )
        .expect("insert setting");
        println!("set {}", args[i]);
        i += 2;
    }
}
