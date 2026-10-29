//! 도킹 레이아웃. 프로젝트·에디터·도구 창을 탭으로 묶고, 탭을 끌어 다른 묶음에 넣거나 옆·위아래로
//! 나눈다. 배치는 설정 폴더의 `layout.json`에 저장해 다음 실행 때 되살린다.

use std::path::Path;

use eframe::egui;
use egui_tiles::{Container, Linear, LinearDir, Tile, Tiles, Tree};
use serde::{Deserialize, Serialize};

/// 창 종류. 종류마다 하나씩만 둔다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pane {
    Project,
    Editor,
    Terminal,
    Problems,
    Search,
    References,
    Debug,
    DebugConsole,
    Sql,
    Schema,
    Plugins,
    Outline,
    Endpoints,
}

/// 열고 닫는 도구 창. 프로젝트와 에디터는 늘 있다.
const TOOLS: [Pane; 11] = [
    Pane::Terminal,
    Pane::Problems,
    Pane::Search,
    Pane::References,
    Pane::Debug,
    Pane::DebugConsole,
    Pane::Sql,
    Pane::Schema,
    Pane::Plugins,
    Pane::Outline,
    Pane::Endpoints,
];

const ALL: [Pane; 13] = [
    Pane::Project,
    Pane::Editor,
    Pane::Terminal,
    Pane::Problems,
    Pane::Search,
    Pane::References,
    Pane::Debug,
    Pane::DebugConsole,
    Pane::Sql,
    Pane::Schema,
    Pane::Plugins,
    Pane::Outline,
    Pane::Endpoints,
];

impl Pane {
    pub fn title(self) -> &'static str {
        match self {
            Pane::Project => "프로젝트",
            Pane::Editor => "에디터",
            Pane::Terminal => "터미널",
            Pane::Problems => "문제",
            Pane::Search => "검색",
            Pane::References => "참조",
            Pane::Debug => "디버그",
            Pane::DebugConsole => "디버그 출력",
            Pane::Sql => "SQL",
            Pane::Schema => "스키마",
            Pane::Plugins => "플러그인",
            Pane::Outline => "개요",
            Pane::Endpoints => "엔드포인트",
        }
    }

    pub fn closable(self) -> bool {
        TOOLS.contains(&self)
    }
}

pub type Layout = Tree<Pane>;

/// 왼쪽에 프로젝트, 오른쪽에 에디터. 도구 창은 처음 열 때 에디터 아래에 붙는다.
pub fn default_layout() -> Layout {
    let mut tiles = Tiles::default();
    let project = tiles.insert_pane(Pane::Project);
    let project = tiles.insert_tab_tile(vec![project]);
    let editor = tiles.insert_pane(Pane::Editor);
    let editor = tiles.insert_tab_tile(vec![editor]);
    let root = tiles.insert_new(Tile::Container(Container::Linear(Linear::new_binary(
        LinearDir::Horizontal,
        [project, editor],
        0.22,
    ))));
    Tree::new(egui::Id::unique("dock"), root, tiles)
}

/// 창이 앞에 보이는지(배치에 있고, 묶음에서 고른 탭인지).
pub fn is_shown(layout: &Layout, pane: Pane) -> bool {
    layout
        .tiles
        .find_pane(&pane)
        .is_some_and(|id| layout.active_tiles().contains(&id))
}

/// 창을 `beside` 창과 같은 묶음에 탭으로 넣어 앞으로 가져온다(개요는 프로젝트 옆). 이미 있으면
/// 그 자리에서 앞으로 가져온다. 그 묶음이 없으면 `show`와 같다.
pub fn show_beside(layout: &mut Layout, pane: Pane, beside: Pane) {
    if layout.tiles.find_pane(&pane).is_none()
        && let Some(parent) = layout
            .tiles
            .find_pane(&beside)
            .and_then(|id| layout.tiles.parent_of(id))
        && matches!(
            layout.tiles.get(parent),
            Some(Tile::Container(Container::Tabs(_)))
        )
    {
        let id = layout.tiles.insert_pane(pane);
        if let Some(Tile::Container(Container::Tabs(tabs))) = layout.tiles.get_mut(parent) {
            tabs.add_child(id);
            tabs.set_active(id);
        }
        return;
    }
    show(layout, pane);
}

/// 창을 앞으로 가져온다. 없으면 다른 도구 창이 있는 묶음에 탭으로 넣고, 그런 묶음도 없으면
/// 에디터 묶음을 위아래로 나눠 아래쪽에 새 묶음을 둔다.
pub fn show(layout: &mut Layout, pane: Pane) {
    if let Some(id) = layout.tiles.find_pane(&pane) {
        layout.make_active(|tile, _| tile == id);
        return;
    }
    let tools = TOOLS
        .iter()
        .filter_map(|p| layout.tiles.find_pane(p))
        .filter_map(|id| layout.tiles.parent_of(id))
        .find(|&parent| {
            matches!(
                layout.tiles.get(parent),
                Some(Tile::Container(Container::Tabs(_)))
            )
        });
    if let Some(parent) = tools {
        let id = layout.tiles.insert_pane(pane);
        if let Some(Tile::Container(Container::Tabs(tabs))) = layout.tiles.get_mut(parent) {
            tabs.add_child(id);
            tabs.set_active(id);
        }
        return;
    }
    // 에디터는 늘 있다(불러올 때 확인하고, 닫지 못한다).
    let Some(editor) = layout.tiles.find_pane(&Pane::Editor) else {
        return;
    };
    let anchor = layout.tiles.parent_of(editor).unwrap_or(editor);
    // 새 나눔도 anchor를 자식으로 가지므로, 원래 부모는 나눔을 넣기 전에 찾아 둔다.
    let parent = layout.tiles.parent_of(anchor);
    let id = layout.tiles.insert_pane(pane);
    let tabs = layout.tiles.insert_tab_tile(vec![id]);
    let split = layout
        .tiles
        .insert_new(Tile::Container(Container::Linear(Linear::new_binary(
            LinearDir::Vertical,
            [anchor, tabs],
            0.7,
        ))));
    match parent.and_then(|p| layout.tiles.get_mut(p)) {
        Some(Tile::Container(container)) => {
            let _ = container.replace_child(anchor, split);
        }
        _ => layout.root = Some(split),
    }
}

pub fn hide(layout: &mut Layout, pane: Pane) {
    if let Some(id) = layout.tiles.find_pane(&pane) {
        layout.remove_recursively(id);
    }
}

/// 앞에 보이면 닫고, 아니면(닫혔거나 다른 탭 뒤에 있으면) 앞으로 가져온다.
pub fn toggle(layout: &mut Layout, pane: Pane) {
    if is_shown(layout, pane) {
        hide(layout, pane);
    } else {
        show(layout, pane);
    }
}

/// 저장한 배치를 읽는다. 없거나 깨졌거나 꼭 있어야 할 창이 빠졌으면 기본 배치.
pub fn load(path: Option<&Path>) -> Layout {
    let mut layout = path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<Layout>(&text).ok())
        .filter(is_valid)
        .unwrap_or_else(default_layout);
    // 참조 결과는 저장하지 않으므로, 빈 참조 창이 남지 않게 뺀다.
    hide(&mut layout, Pane::References);
    layout
}

/// 창 종류마다 많아야 하나, 프로젝트와 에디터는 꼭 하나.
fn is_valid(layout: &Layout) -> bool {
    let count = |pane: Pane| {
        layout
            .tiles
            .tiles()
            .filter(|tile| matches!(tile, Tile::Pane(p) if *p == pane))
            .count()
    };
    layout.root.is_some()
        && ALL.iter().all(|&pane| {
            let n = count(pane);
            if pane.closable() { n <= 1 } else { n == 1 }
        })
}

pub fn save(path: &Path, layout: &Layout) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string(layout).map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent(layout: &Layout, pane: Pane) -> egui_tiles::TileId {
        let id = layout.tiles.find_pane(&pane).unwrap();
        layout.tiles.parent_of(id).unwrap()
    }

    #[test]
    fn 도구_창은_에디터_아래_한_묶음에_탭으로_모인다() {
        let mut layout = default_layout();
        assert!(!is_shown(&layout, Pane::Terminal));

        show(&mut layout, Pane::Terminal);
        assert!(is_shown(&layout, Pane::Terminal));
        let split = layout
            .tiles
            .parent_of(parent(&layout, Pane::Terminal))
            .unwrap();
        assert!(matches!(
            layout.tiles.get(split),
            Some(Tile::Container(Container::Linear(l))) if l.dir == LinearDir::Vertical
        ));
        assert_eq!(
            layout.tiles.parent_of(parent(&layout, Pane::Editor)),
            Some(split),
            "에디터 묶음과 위아래로 나뉜다"
        );

        // 둘째 도구 창은 같은 묶음의 앞 탭이 되고, 터미널은 뒤로 간다.
        show(&mut layout, Pane::Search);
        assert_eq!(
            parent(&layout, Pane::Search),
            parent(&layout, Pane::Terminal)
        );
        assert!(is_shown(&layout, Pane::Search));
        assert!(!is_shown(&layout, Pane::Terminal));

        // 뒤에 있는 창을 토글하면 닫지 않고 앞으로 가져온다. 앞에 있으면 닫는다.
        toggle(&mut layout, Pane::Terminal);
        assert!(is_shown(&layout, Pane::Terminal));
        toggle(&mut layout, Pane::Terminal);
        assert!(layout.tiles.find_pane(&Pane::Terminal).is_none());
    }

    #[test]
    fn 저장한_배치를_되살리고_깨졌으면_기본_배치로() {
        let dir = std::env::temp_dir().join(format!("ferrite-dock-{}", std::process::id()));
        let path = dir.join("layout.json");
        let mut layout = default_layout();
        show(&mut layout, Pane::Terminal);
        show(&mut layout, Pane::References);
        save(&path, &layout).unwrap();

        let loaded = load(Some(&path));
        assert!(loaded.tiles.find_pane(&Pane::Terminal).is_some());
        assert!(
            loaded.tiles.find_pane(&Pane::References).is_none(),
            "참조 결과는 저장하지 않으므로 창도 뺀다"
        );

        std::fs::write(&path, "{").unwrap();
        assert!(load(Some(&path)).tiles.find_pane(&Pane::Terminal).is_none());

        // 에디터가 빠진 배치는 받지 않는다.
        let mut broken = default_layout();
        let editor = broken.tiles.find_pane(&Pane::Editor).unwrap();
        broken.remove_recursively(editor);
        save(&path, &broken).unwrap();
        assert!(load(Some(&path)).tiles.find_pane(&Pane::Editor).is_some());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
