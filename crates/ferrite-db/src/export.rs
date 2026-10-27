//! CSV로 내보내기. 조회를 따로 연 연결에서 다시 실행해 한 페이지씩 파일에 흘려 쓴다(그리드 버퍼와 따로,
//! 메모리는 한 페이지만큼). Excel이 한글을 UTF-8로 읽도록 BOM을 붙인다. 다 쓰면 이름을 바꿔, 멈추거나
//! 실패하면 쓰던 파일을 지운다.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::{Cell, Connection, ConnectionConfig, Event};

/// 칸 하나를 CSV 필드로. NULL은 빈칸이다.
pub fn csv_field(cell: &Cell) -> String {
    let text = match cell {
        Cell::Null => return String::new(),
        other => other.to_string(),
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// `sql`(조회 하나)을 실행해 `path`에 쓴다. 쓴 행 수를 돌려준다. `progress`는 페이지마다 쓴 행 수로 불린다.
pub fn export_csv(
    config: &ConnectionConfig,
    password: Option<String>,
    sql: &str,
    path: &Path,
    stop: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<u64, String> {
    let part = path.with_extension("csv.part");
    let result = write_csv(config, password, sql, &part, stop, progress);
    match &result {
        Ok(_) => std::fs::rename(&part, path).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(_) => {
            let _ = std::fs::remove_file(&part);
        }
    }
    result
}

fn write_csv(
    config: &ConnectionConfig,
    password: Option<String>,
    sql: &str,
    part: &Path,
    stop: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<u64, String> {
    let file = File::create(part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut out = BufWriter::new(file);
    let io = |e: std::io::Error| e.to_string();
    out.write_all("\u{feff}".as_bytes()).map_err(io)?;
    let conn = Connection::open(config, password, || {});
    conn.execute(sql);
    let mut written = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            conn.cancel();
            return Err("내보내기를 멈췄습니다".into());
        }
        let Some(event) = conn.next_event(Duration::from_millis(200)) else {
            continue;
        };
        match event {
            Event::Columns(columns) => {
                let header: Vec<String> = columns
                    .into_iter()
                    .map(|c| csv_field(&Cell::Text(c.name)))
                    .collect();
                writeln!(out, "{}", header.join(",")).map_err(io)?;
            }
            Event::Rows { rows, more } => {
                for row in &rows {
                    let fields: Vec<String> = row.iter().map(csv_field).collect();
                    writeln!(out, "{}", fields.join(",")).map_err(io)?;
                }
                written += rows.len() as u64;
                progress(written);
                if more {
                    conn.fetch();
                }
            }
            Event::Done { .. } => break,
            Event::Error(e) | Event::Disconnected(e) => return Err(e),
            _ => {}
        }
    }
    out.flush().map_err(io)?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_필드를_필요할_때만_따옴표로_감싼다() {
        assert_eq!(csv_field(&Cell::Null), "");
        assert_eq!(csv_field(&Cell::Int(3)), "3");
        assert_eq!(csv_field(&Cell::Text("한글".into())), "한글");
        assert_eq!(csv_field(&Cell::Text("a,b".into())), "\"a,b\"");
        assert_eq!(
            csv_field(&Cell::Text("say \"hi\"\n".into())),
            "\"say \"\"hi\"\"\n\""
        );
    }
}
