//! Connections navigator (tree) and the New / Edit Database Connection dialog.

use crate::conn::{Conn, ConnRef};
use rsqldev_core::config::{self, ConnConfig};
use rsqldev_core::dialect;
use rsqldev_core::registry::{self, Kind};
use std::collections::BTreeMap;
use wxdragon::prelude::*;

#[derive(Clone, Debug)]
pub enum Node {
    Folder,
    Conn(String),
    Schemas { conn: String },
    Schema { conn: String, owner: String },
    Kind { conn: String, owner: String, kind: &'static str },
    Obj { conn: String, owner: String, kind: &'static str, name: String },
    Leaf,
    Loading,
}

#[derive(Clone, Copy)]
pub struct Navigator {
    pub panel: Panel,
    pub tree: TreeCtrl,
}

const M_CONNECT: i32 = 7101;
const M_DISCONNECT: i32 = 7102;
const M_WORKSHEET: i32 = 7103;
const M_PROPS: i32 = 7104;
const M_DELETE: i32 = 7105;
const M_REFRESH: i32 = 7106;
const M_OPEN: i32 = 7107;
const M_DROP: i32 = 7109;
const M_DDL: i32 = 7111;
const M_COMPILE: i32 = 7112;
const M_NEW_TABLE: i32 = 7114;
const M_EDIT_TABLE: i32 = 7115;
const M_NEW_INDEX: i32 = 7116;
const M_REBUILD: i32 = 7117;
const M_FILTER: i32 = 7118;
const M_CLEAR_FILTER: i32 = 7119;
const M_DDL_CLIP: i32 = 7120;
const M_DDL_FILE: i32 = 7121;
const M_RENAME_CONN: i32 = 7122;
const M_DUP_CONN: i32 = 7123;
const M_RECONNECT: i32 = 7124;

/// Saved name filter of a tree folder (Apply Filter…).
fn filter_of(conn: &str, owner: &str, kind: &str) -> Option<String> {
    crate::app().conns.borrow().iter().find(|c| c.name == conn)?.filters.get(&config::filter_key(owner, kind)).cloned()
}

impl Navigator {
    pub fn new(parent: &dyn WxWidget) -> Navigator {
        let panel = Panel::builder(parent).build();
        let bar = BoxSizer::builder(Orientation::Horizontal).build();
        let add = crate::icons::tool(&panel, "new", "New Connection…");
        let refresh = crate::icons::tool(&panel, "refresh", "Refresh");
        let filter = crate::icons::tool(&panel, "filter", "Apply Filter… (on Tables, Views, …)");
        let collapse = crate::icons::tool(&panel, "collapse", "Collapse All");
        bar.add(&add, 0, SizerFlag::All, 3);
        bar.add(&refresh, 0, SizerFlag::All, 3);
        bar.add(&filter, 0, SizerFlag::All, 3);
        bar.add(&collapse, 0, SizerFlag::All, 3);
        let tree = TreeCtrl::builder(&panel).with_style(TreeCtrlStyle::Default | TreeCtrlStyle::HideRoot).build();
        tree.set_image_list(crate::icons::tree_images());
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
        s.add(&tree, 1, SizerFlag::Expand, 0);
        panel.set_sizer(s, true);
        let nav = Navigator { panel, tree };
        add.on_click(move |_| crate::app().new_connection());
        collapse.on_click(move |_| tree.collapse_all());
        filter.on_click(move |_| match tree.get_selection().map(|i| (nav.node(&i), i)) {
            Some((Node::Kind { conn, owner, kind }, i)) => nav.set_filter(&i, &conn, &owner, kind, true),
            _ => crate::ui::info(&tree, "Select a folder (Tables, Views, …) under a connection to filter it."),
        });
        refresh.on_click(move |_| {
            if let Some(i) = tree.get_selection() {
                nav.refresh(&i);
            }
        });
        tree.on_item_expanding(move |e| {
            if let Some(i) = e.get_item() {
                nav.expanding(&i);
            }
        });
        tree.on_item_activated(move |e| {
            if let Some(i) = e.get_item() {
                nav.activated(&i);
            }
        });
        tree.on_item_right_click(move |e| {
            if let Some(i) = e.get_item() {
                tree.select_item(&i);
                nav.context_menu(&i);
            }
        });
        // Ctrl+C / Cmd+C copies the selected node's name (connection, table, column, ...).
        // Registered on focus and on the user's selecting; rebuilds from code run quietly.
        let copy_target = move || {
            crate::copy::set(move || {
                if !tree.is_valid() {
                    return false;
                }
                if let Some(t) = tree.get_selection().and_then(|i| tree.get_item_text(&i)).filter(|t| !t.is_empty()) {
                    Clipboard::get().set_text(&t);
                }
                true
            })
        };
        tree.on_set_focus(move |_| copy_target());
        tree.on_selection_changed(move |_| copy_target());
        nav
    }

    /// Expand the first connection and its Tables folder (self-test screenshots).
    pub fn expand_first(&self) {
        let Some(root) = self.tree.get_root_item() else { return };
        if let Some((c, _)) = self.tree.get_first_child(&root) {
            self.tree.expand(&c);
            crate::copy::quietly(|| self.tree.select_item(&c));
        }
    }

    /// Select `owner.name` in the tree, expanding (and loading) the path to it: the object
    /// viewer's "select in navigator", like IntelliJ's select-in-project. Gives up quietly
    /// after 15 s or when a step is missing.
    pub fn reveal(&self, conn: &str, schema: &str, owner: &str, kind: &'static str, name: &str) {
        let nav = *self;
        let (conn, owner, name) = (conn.to_string(), owner.to_string(), name.to_string());
        // Own schema: folders right under the connection; other schemas under Schemas ▸ owner.
        let own = owner.eq_ignore_ascii_case(schema);
        let steps: Vec<Box<dyn Fn(&Node) -> bool>> = {
            let (c1, c2, o1, o2, n1) = (conn.clone(), conn.clone(), owner.clone(), owner.clone(), name.clone());
            let mut v: Vec<Box<dyn Fn(&Node) -> bool>> = vec![Box::new(move |n| matches!(n, Node::Conn(c) if *c == c1))];
            if !own {
                v.push(Box::new(move |n| matches!(n, Node::Schemas { conn } if *conn == c2)));
                v.push(Box::new(move |n| matches!(n, Node::Schema { owner, .. } if owner.eq_ignore_ascii_case(&o1))));
            }
            v.push(Box::new(move |n| matches!(n, Node::Kind { kind: k, owner, .. } if *k == kind && owner.eq_ignore_ascii_case(&o2))));
            v.push(Box::new(move |n| matches!(n, Node::Obj { name: x, .. } if *x == n1)));
            v
        };
        let Some(root) = self.tree.get_root_item() else { return };
        let (mut at, mut step) = (root, 0usize);
        let start = std::time::Instant::now();
        crate::hub::every_tick(move || {
            if !nav.tree.is_valid() || start.elapsed().as_secs() > 15 {
                return false;
            }
            // Wait while a load replaces the Loading placeholder.
            if nav.first_child_is_loading(&at) {
                if !nav.tree.is_expanded(&at) {
                    nav.tree.expand(&at);
                }
                return true;
            }
            // Connections can sit in folders: look one folder level deep at the top.
            let found = nav.child_where(&at, &steps[step]).or_else(|| {
                (step == 0).then(|| nav.children(&at).into_iter().filter(|c| matches!(nav.node(c), Node::Folder)).find_map(|f| nav.child_where(&f, &steps[0]))).flatten()
            });
            let Some(next) = found else { return false };
            step += 1;
            if step == steps.len() {
                crate::copy::quietly(|| nav.tree.select_item(&next));
                nav.tree.ensure_visible(&next);
                return false;
            }
            nav.tree.expand(&next);
            at = next;
            true
        });
    }

    fn children(&self, i: &TreeItemId) -> Vec<TreeItemId> {
        let mut out = vec![];
        if let Some((mut c, _)) = self.tree.get_first_child(i) {
            loop {
                out.push(c.clone());
                match self.tree.get_next_sibling(&c) {
                    Some(n) => c = n,
                    None => break,
                }
            }
        }
        out
    }

    fn child_where(&self, i: &TreeItemId, f: &dyn Fn(&Node) -> bool) -> Option<TreeItemId> {
        self.children(i).into_iter().find(|c| f(&self.node(c)))
    }

    fn node(&self, i: &TreeItemId) -> Node {
        self.tree.get_custom_data(i).and_then(|d| d.downcast_ref::<Node>().cloned()).unwrap_or(Node::Leaf)
    }

    /// Tree image of connection `name`: its engine's icon, with the "open" badge when `up`.
    fn conn_image(name: &str, up: bool) -> i32 {
        let driver = crate::app().conns.borrow().iter().find(|c| c.name == name).map(|c| c.driver.clone()).unwrap_or_default();
        crate::icons::tree_index(&crate::icons::conn_icon(&driver, up))
    }

    fn set_conn_image(&self, i: &TreeItemId, name: &str, up: bool) {
        let img = Self::conn_image(name, up);
        self.tree.set_item_image(i, img, TreeItemIcon::Normal);
        self.tree.set_item_image(i, img, TreeItemIcon::Selected);
    }

    fn add(&self, parent: &TreeItemId, text: &str, n: Node, expandable: bool) -> Option<TreeItemId> {
        let icon = match &n {
            Node::Conn(_) => "",
            Node::Folder => "folder",
            Node::Schemas { .. } | Node::Schema { .. } => "user",
            Node::Kind { kind, .. } => crate::icons::for_kind(kind),
            Node::Obj { kind, .. } => crate::icons::for_kind(kind),
            Node::Leaf => "column",
            Node::Loading => "",
        };
        let img = match &n {
            Node::Conn(name) => Self::conn_image(name, crate::app().open_conn(name).is_some()),
            _ => crate::icons::tree_index(icon),
        };
        let img = Some(img).filter(|i| *i >= 0);
        let i = self.tree.append_item_with_data(parent, text, n, img, img)?;
        if expandable {
            self.tree.append_item_with_data(&i, "Loading…", Node::Loading, None, None);
        }
        Some(i)
    }

    /// Rebuild the top level from saved connections, grouped by folder.
    pub fn reload(&self, conns: &[ConnConfig]) {
        crate::copy::quietly(|| self.rebuild(conns));
    }

    fn rebuild(&self, conns: &[ConnConfig]) {
        let t = self.tree;
        t.delete_all_items();
        let Some(root) = t.add_root("Connections", None, None) else { return };
        let mut folders: BTreeMap<&str, TreeItemId> = BTreeMap::new();
        let mut sorted: Vec<&ConnConfig> = conns.iter().collect();
        sorted.sort_by_key(|c| (c.folder.to_lowercase(), c.name.to_lowercase()));
        for c in sorted {
            let parent = if c.folder.is_empty() {
                root.clone()
            } else {
                folders.entry(&c.folder).or_insert_with(|| self.add(&root, &c.folder, Node::Folder, false).unwrap()).clone()
            };
            self.add(&parent, &c.name, Node::Conn(c.name.clone()), true);
        }
    }

    fn first_child_is_loading(&self, i: &TreeItemId) -> bool {
        self.tree.get_first_child(i).is_some_and(|(c, _)| matches!(self.node(&c), Node::Loading))
    }

    fn expanding(&self, i: &TreeItemId) {
        if !self.first_child_is_loading(i) {
            return;
        }
        let nav = *self;
        let item = i.clone();
        match self.node(i) {
            Node::Conn(name) => crate::app().connect(&name.clone(), move |c| {
                let owner = c.borrow().schema.clone();
                crate::copy::quietly(|| {
                    nav.tree.delete_children(&item);
                    nav.set_conn_image(&item, &name, true);
                    nav.add_folders(&item, &name, &owner, c.borrow().kind);
                    nav.add(&item, dialect::schemas_label(c.borrow().kind), Node::Schemas { conn: name.clone() }, true);
                    nav.tree.expand(&item);
                });
            }),
            Node::Schema { conn, owner } => {
                if let Some(c) = crate::app().open_conn(&conn) {
                    let kind = c.borrow().kind;
                    self.tree.delete_children(i);
                    self.add_folders(i, &conn, &owner, kind);
                }
            }
            Node::Schemas { conn } => self.load(i, &conn.clone(), None, move |nav, item, kind, rows| {
                for r in rows {
                    if let Some(o) = r.first().cloned().flatten() {
                        nav.add(item, &o, Node::Schema { conn: conn.clone(), owner: o.clone() }, true);
                    }
                }
                let _ = kind;
            }),
            Node::Kind { conn, owner, kind } => {
                let c = crate::app().open_conn(&conn);
                let db = c.map(|c| c.borrow().kind).unwrap_or(Kind::Sqlite);
                let sql = dialect::list_sql(db, kind, &owner);
                let pat = filter_of(&conn, &owner, kind);
                self.load(i, &conn.clone(), sql, move |nav, item, db, rows| {
                    let expandable = dialect::children_sql(db, kind).is_some() || dialect::extra_children_sql(db, kind).is_some();
                    let red = crate::theme::c(crate::theme::p().invalid);
                    for r in rows {
                        let Some(name) = r.first().cloned().flatten() else { continue };
                        // ponytail: filtered client side (the full list is fetched, only matches go
                        // into the tree); push the pattern into list_sql if huge schemas need it.
                        if pat.as_deref().is_some_and(|p| !config::name_matches(p, &name)) {
                            continue;
                        }
                        let n = Node::Obj { conn: conn.clone(), owner: owner.clone(), kind, name: name.clone() };
                        if let Some(ni) = nav.add(item, &name, n, expandable) {
                            if r.get(1).and_then(|s| s.as_deref()) == Some("INVALID") {
                                nav.tree.set_item_text_colour(&ni, red);
                            }
                        }
                    }
                });
            }
            Node::Obj { conn, owner, kind, name } => {
                let c = crate::app().open_conn(&conn);
                let db = c.map(|c| c.borrow().kind).unwrap_or(Kind::Sqlite);
                let extra = dialect::extra_children_sql(db, kind).map(|s| dialect::fill(db, s, &owner, &name));
                let leaves = |nav: &Navigator, item: &TreeItemId, rows: Vec<Vec<rsqldev_core::Cell>>| {
                    for r in rows {
                        let name = r.first().cloned().flatten().unwrap_or_default();
                        let detail = r.get(1).cloned().flatten().unwrap_or_default();
                        let text = if detail.trim().is_empty() { name } else { format!("{name} ({})", detail.trim().to_uppercase()) };
                        nav.add(item, &text, Node::Leaf, false);
                    }
                };
                // Columns first (if any), then members / arguments / partitions.
                match dialect::children_sql(db, kind).map(|s| dialect::fill(db, &s, &owner, &name)) {
                    Some(cols) => self.load(i, &conn.clone(), Some(cols), move |nav, item, _, rows| {
                        leaves(nav, item, rows);
                        if let (Some(extra), Some(c)) = (extra, crate::app().open_conn(&conn)) {
                            let (nav, item) = (*nav, item.clone());
                            Conn::meta_query(&c, &extra, move |r| {
                                if let Ok(r) = r {
                                    crate::copy::quietly(|| leaves(&nav, &item, r.rows));
                                }
                            });
                        }
                    }),
                    None => self.load(i, &conn, extra, move |nav, item, _, rows| leaves(nav, item, rows)),
                }
            }
            _ => {}
        }
    }

    fn add_folders(&self, parent: &TreeItemId, conn: &str, owner: &str, kind: Kind) {
        for (label, k) in dialect::folders(kind) {
            let text = if filter_of(conn, owner, k).is_some() { format!("{label} (Filtered)") } else { label.to_string() };
            self.add(parent, &text, Node::Kind { conn: conn.into(), owner: owner.into(), kind: k }, true);
        }
    }

    /// Run a metadata query and replace the Loading placeholder with `fill`'s children.
    fn load(&self, i: &TreeItemId, conn: &str, sql: Option<String>, fill: impl FnOnce(&Navigator, &TreeItemId, Kind, Vec<Vec<rsqldev_core::Cell>>) + 'static) {
        let Some(c) = crate::app().open_conn(conn) else { return };
        let kind = c.borrow().kind;
        let sql = sql.unwrap_or_else(|| dialect::schemas_sql(kind).to_string());
        let nav = *self;
        let item = i.clone();
        Conn::meta_query(&c, &sql, move |r| {
            crate::copy::quietly(|| {
                nav.tree.delete_children(&item);
                match r {
                    Ok(r) => fill(&nav, &item, kind, r.rows),
                    Err(e) => {
                        nav.add(&item, &format!("Error: {e}"), Node::Leaf, false);
                    }
                }
            });
        });
    }

    pub fn refresh(&self, i: &TreeItemId) {
        match self.node(i) {
            Node::Conn(_) | Node::Schemas { .. } | Node::Schema { .. } | Node::Kind { .. } | Node::Obj { .. } => {
                self.tree.collapse(i);
                self.tree.delete_children(i);
                self.tree.append_item_with_data(i, "Loading…", Node::Loading, None, None);
                self.tree.expand(i);
            }
            _ => {}
        }
    }

    /// Apply Filter… (`ask`) or Clear Filter on a folder: saved with the connection, the folder
    /// reloads with only the matching names and is labelled "(Filtered)".
    fn set_filter(&self, i: &TreeItemId, conn: &str, owner: &str, kind: &'static str, ask: bool) {
        let pat = if ask {
            let cur = filter_of(conn, owner, kind).unwrap_or_default();
            match crate::ui::ask(&self.tree, "Name filter (% or * = any characters, _ = one character, plain text = contains):", "Apply Filter", &cur) {
                Some(p) => p.trim().to_string(),
                None => return,
            }
        } else {
            String::new()
        };
        let app = crate::app();
        {
            let mut cs = app.conns.borrow_mut();
            let Some(c) = cs.iter_mut().find(|c| c.name == conn) else { return };
            let key = config::filter_key(owner, kind);
            if pat.is_empty() {
                c.filters.remove(&key);
            } else {
                c.filters.insert(key, pat.clone());
            }
        }
        // Saved directly: App::save_conns would rebuild (and collapse) the whole tree.
        if let Err(e) = config::save_connections(&app.conns.borrow()) {
            crate::ui::error(&self.tree, &format!("Saving the filter failed: {e}"));
        }
        let base = self.tree.get_item_text(i).unwrap_or_default().trim_end_matches(" (Filtered)").to_string();
        self.tree.set_item_text(i, &if pat.is_empty() { base } else { format!("{base} (Filtered)") });
        self.refresh(i);
    }

    fn activated(&self, i: &TreeItemId) {
        match self.node(i) {
            Node::Conn(name) => crate::app().connect(&name, move |c| crate::app().open_worksheet(c)),
            Node::Obj { conn, owner, kind, name } => crate::app().open_object(&conn, &owner, kind, &name),
            _ => {}
        }
    }

    fn context_menu(&self, i: &TreeItemId) {
        let node = self.node(i);
        let mut b = Menu::builder();
        match &node {
            Node::Conn(name) => {
                let up = crate::app().open_conn(name).is_some();
                b = if up { b.append_item(M_DISCONNECT, "Disconnect", "").append_item(M_RECONNECT, "Reconnect", "") } else { b.append_item(M_CONNECT, "Connect", "") };
                b = b
                    .append_item(M_WORKSHEET, "Open SQL Worksheet", "")
                    .append_separator()
                    .append_item(M_PROPS, "Properties…", "")
                    .append_item(M_RENAME_CONN, "Rename…", "")
                    .append_item(M_DUP_CONN, "Duplicate", "Same settings and saved password under a new name")
                    .append_item(M_DELETE, "Delete Connection", "")
                    .append_item(M_REFRESH, "Refresh", "");
            }
            Node::Obj { kind, conn, .. } => {
                b = b
                    .append_item(M_OPEN, "Open", "")
                    .append_item(M_REFRESH, "Refresh", "")
                    .append_item(M_DDL, "Quick DDL ▸ Worksheet", "")
                    .append_item(M_DDL_CLIP, "Quick DDL ▸ Clipboard", "")
                    .append_item(M_DDL_FILE, "Quick DDL ▸ Save to File…", "")
                    .append_separator();
                if *kind == dialect::TABLE {
                    b = b
                        .append_item(M_EDIT_TABLE, "Edit…", "Columns, keys, constraints, indexes and storage, with the DDL shown first")
                        .append_item(M_NEW_INDEX, "New Index…", "");
                }
                let db = crate::app().open_conn(conn).map(|c| c.borrow().kind);
                // Rebuild needs only the index name on these engines.
                if *kind == dialect::INDEX && matches!(db, Some(Kind::Oracle | Kind::Postgres | Kind::Sqlite)) {
                    b = b.append_item(M_REBUILD, "Rebuild", "");
                }
                let oracle = db == Some(Kind::Oracle);
                if oracle && matches!(*kind, dialect::PACKAGE | dialect::PROCEDURE | dialect::FUNCTION | dialect::TRIGGER | dialect::VIEW | dialect::TYPE) {
                    b = b.append_item(M_COMPILE, "Compile", "");
                }
                if *kind != dialect::TABLE {
                    b = b.append_item(M_DROP, "Drop…", "");
                }
            }
            Node::Kind { conn, owner, kind } => {
                if *kind == dialect::TABLE {
                    b = b.append_item(M_NEW_TABLE, "New Table…", "").append_separator();
                }
                b = b.append_item(M_FILTER, "Apply Filter…", "Show only the names matching a pattern");
                if filter_of(conn, owner, kind).is_some() {
                    b = b.append_item(M_CLEAR_FILTER, "Clear Filter", "");
                }
                b = b.append_separator().append_item(M_REFRESH, "Refresh", "");
            }
            Node::Schemas { .. } | Node::Schema { .. } => b = b.append_item(M_REFRESH, "Refresh", ""),
            _ => return,
        }
        let mut m = b.build();
        if let Node::Obj { kind: dialect::TABLE, conn, .. } = &node {
            if let Some(c) = crate::app().open_conn(conn) {
                crate::table_menu::append(&m, c.borrow().kind);
            }
        }
        let nav = *self;
        let item = i.clone();
        m.on_selected(move |e| nav.menu(e.get_id(), &item, node.clone()));
        self.tree.popup_menu(&mut m, None);
        m.destroy_menu();
    }

    fn menu(&self, id: i32, i: &TreeItemId, node: Node) {
        let app = crate::app();
        match (id, node) {
            (M_REFRESH, _) => self.refresh(i),
            (M_FILTER | M_CLEAR_FILTER, Node::Kind { conn, owner, kind }) => self.set_filter(i, &conn, &owner, kind, id == M_FILTER),
            (M_CONNECT, Node::Conn(_)) => self.tree.expand(i),
            (M_DISCONNECT, Node::Conn(name)) => {
                let (nav, i) = (*self, i.clone());
                app.end_work(Some(&name.clone()), move || {
                    crate::app().disconnect(&name);
                    nav.set_conn_image(&i, &name, false);
                    nav.tree.collapse(&i);
                    nav.tree.delete_children(&i);
                    nav.tree.append_item_with_data(&i, "Loading…", Node::Loading, None, None);
                });
            }
            (M_RENAME_CONN, Node::Conn(name)) => app.rename_connection(&name),
            (M_DUP_CONN, Node::Conn(name)) => app.duplicate_connection(&name),
            (M_RECONNECT, Node::Conn(name)) => {
                let (nav, i) = (*self, i.clone());
                app.end_work(Some(&name.clone()), move || {
                    crate::app().disconnect(&name);
                    // Expanding the Loading placeholder connects again.
                    nav.refresh(&i);
                });
            }
            (M_WORKSHEET, Node::Conn(name)) => app.connect(&name, move |c| crate::app().open_worksheet(c)),
            (M_PROPS, Node::Conn(name)) => app.edit_connection(&name),
            (M_DELETE, Node::Conn(name)) => {
                if crate::ui::confirm(&self.tree, &format!("Delete connection \"{name}\"?")) {
                    app.delete_connection(&name);
                }
            }
            (id, Node::Obj { conn, owner, name, .. }) if crate::table_menu::handles(id) => {
                let Some(c) = app.open_conn(&conn) else { return };
                // Refresh the table list (renames, drops, copies) once the statements ran.
                let (nav, item) = (*self, i.clone());
                let done: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(move || {
                    if let Some(p) = nav.tree.get_item_parent(&item) {
                        nav.refresh(&p);
                    }
                });
                crate::table_menu::run(id, c, owner, name, done);
            }
            (M_OPEN, Node::Obj { conn, owner, kind, name }) => app.open_object(&conn, &owner, kind, &name),
            (M_DDL | M_DDL_CLIP | M_DDL_FILE, Node::Obj { conn, owner, kind, name }) => {
                let to = match id {
                    M_DDL_CLIP => crate::QuickDdl::Clipboard,
                    M_DDL_FILE => crate::QuickDdl::File,
                    _ => crate::QuickDdl::Worksheet,
                };
                app.quick_ddl(&conn, &owner, kind, &name, to);
            }
            (M_NEW_TABLE, Node::Kind { conn, owner, .. }) => {
                let Some(c) = app.open_conn(&conn) else { return };
                let (nav, item) = (*self, i.clone());
                crate::table_editor::create(c, owner, move || nav.refresh(&item));
            }
            (M_EDIT_TABLE | M_NEW_INDEX, Node::Obj { conn, owner, name, .. }) => {
                let Some(c) = app.open_conn(&conn) else { return };
                let (nav, item) = (*self, i.clone());
                let start = if id == M_NEW_INDEX { crate::table_editor::Start::Indexes } else { crate::table_editor::Start::Columns };
                crate::table_editor::edit(c, owner, name, start, move || nav.refresh(&item));
            }
            (_, Node::Obj { conn, owner, kind, name }) => {
                let Some(c) = app.open_conn(&conn) else { return };
                let db = c.borrow().kind;
                let q = dialect::qualified(db, &owner, &name);
                let sql = match id {
                    M_DROP => {
                        let stmt = match (db, kind) {
                            (Kind::Postgres, dialect::FUNCTION | dialect::PROCEDURE) => format!("DROP {kind} {}.{name}", dialect::quote(db, &owner)),
                            (Kind::Postgres, dialect::TRIGGER) => return crate::ui::info(&self.tree, "Drop the trigger from its table's Triggers tab (needs ON table)."),
                            // Oracle names a database link bare (dots belong to the name) and
                            // only its owner can drop it.
                            (Kind::Oracle, dialect::DBLINK) if !owner.eq_ignore_ascii_case(&c.borrow().schema) => {
                                return crate::ui::info(&self.tree, &format!("Only its owner can drop a database link: connect as {owner}."));
                            }
                            (Kind::Oracle, dialect::DBLINK) if name.chars().all(|ch| ch.is_ascii_alphanumeric() || "_$#.@".contains(ch)) => format!("DROP DATABASE LINK {name}"),
                            (Kind::Oracle, dialect::DBLINK) => return crate::ui::info(&self.tree, "This database link name needs to be dropped by hand."),
                            _ => format!("DROP {kind} {q}"),
                        };
                        if !crate::ui::confirm_sql(&self.tree, &format!("Drop {kind} {q}?"), &stmt) {
                            return;
                        }
                        stmt
                    }
                    M_COMPILE => format!("ALTER {} {q} COMPILE", if kind == dialect::PACKAGE { "PACKAGE" } else { kind }),
                    M_REBUILD => match rsqldev_core::ddl::rebuild_index(db, &owner, "", &name) {
                        Some(s) if crate::ui::confirm_sql(&self.tree, &format!("Rebuild index {q}?"), &s) => s,
                        _ => return,
                    },
                    _ => return,
                };
                let nav = *self;
                let item = i.clone();
                Conn::work_query(&c, &sql, move |r| match r {
                    Ok(_) => {
                        if let Some(p) = nav.tree.get_item_parent(&item) {
                            nav.refresh(&p);
                        }
                    }
                    Err(e) => crate::ui::error(&nav.tree, &e),
                });
            }
            _ => {}
        }
    }
}

fn details(c: &ConnConfig) -> String {
    let d = registry::find(&c.driver);
    let port = c.port.or(d.and_then(|d| d.port)).map(|p| p.to_string()).unwrap_or_default();
    match d.map(|d| d.backend) {
        Some(k) if k.is_file() => c.database.clone(),
        Some(Kind::Oracle) if c.conn_type == "tns" || c.conn_type == "custom" => format!("{}@{}", c.user, c.tns),
        Some(Kind::Oracle) if c.sid => format!("{}@{}:{port}:{}", c.user, c.host, c.database),
        Some(Kind::Oracle) => format!("{}@//{}:{port}/{}", c.user, c.host, c.database),
        _ => format!("{}@{}:{port}/{}", c.user, c.host, c.database),
    }
}

/// Result of the connection dialog: (original name if editing, config, password, connect now).
pub type DialogResult = (Option<String>, ConnConfig, String, bool);

/// New / Select Database Connection dialog, laid out like SQL Developer's: saved connections on
/// the left, the form on the right, Save / Clear / Test / Connect / Cancel at the bottom.
pub fn edit_dialog(parent: &Frame, existing: Option<&ConnConfig>, saved: &[ConnConfig]) -> Option<DialogResult> {
    let blank = || ConnConfig { driver: "oracle".into(), conn_type: "basic".into(), save_password: true, host: "localhost".into(), ..Default::default() };
    let dlg = Dialog::builder(parent, "New / Select Database Connection").with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(900, 600).build();

    let list = ListCtrl::builder(&dlg).with_style(ListCtrlStyle::Report | ListCtrlStyle::SingleSel).build();
    list.set_image_list(crate::icons::tree_images(), 1 /* image_list_type::SMALL */);
    list.insert_column(0, "Connection Name", ListColumnFormat::Left, 150);
    list.insert_column(1, "Connection Details", ListColumnFormat::Left, 200);
    for (i, c) in saved.iter().enumerate() {
        list.insert_item(i as i64, &c.name, Some(crate::icons::tree_index(&crate::icons::conn_icon(&c.driver, false))));
        list.set_item_text_by_column(i as i64, 1, &details(c));
    }

    let text = || TextCtrl::builder(&dlg).build();
    let name = text();
    let drivers = ordered_drivers();
    // Engine icons, as in the Connections tree.
    let dbtype = BitmapComboBox::builder(&dlg).with_style(ComboBoxStyle::ReadOnly).build();
    for d in &drivers {
        let label = if POPULAR.contains(&d.id.as_str()) { d.label.clone() } else { format!("{}  ({}-compatible)", d.label, d.category) };
        dbtype.append(&label, crate::icons::bitmap(&crate::icons::conn_icon(&d.id, false), 16).as_ref());
    }
    let folder = text();
    let user = text();
    let pw = TextCtrl::builder(&dlg).with_style(TextCtrlStyle::Password).build();
    let save_pw = CheckBox::builder(&dlg).with_label("Save Password").build();
    const ROLES: [&str; 7] = ["", "SYSDBA", "SYSOPER", "SYSASM", "SYSBACKUP", "SYSDG", "SYSKM"];
    let role = Choice::builder(&dlg).with_choices(ROLES.iter().map(|r| if r.is_empty() { "default".to_string() } else { r.to_string() }).collect()).build();
    let ctype = Choice::builder(&dlg).with_choices(vec!["Basic".into(), "TNS".into(), "Custom JDBC / Easy Connect".into()]).build();
    let host = text();
    let port = text();
    let r_sid = RadioButton::builder(&dlg).with_label("SID").first_in_group().build();
    let sid = text();
    let r_svc = RadioButton::builder(&dlg).with_label("Service name").build();
    let svc = text();
    let db = text();
    let file = text();
    let browse = Button::builder(&dlg).with_label("Browse…").build();
    let tns = text();
    let opts = text();
    opts.set_tooltip("Driver options, e.g. ssh=user@bastion[:22]; ssh_key=~/.ssh/id_ed25519 (SSH tunnel); sslmode=verify-full; trust_cert=true (SQL Server self-signed); ssl=true; verify=false (MySQL); https=true; allow_http=true (ClickHouse); tns_admin=/path");
    let status = StaticText::builder(&dlg).with_label("Status :").build();

    let top = FlexGridSizer::builder(0, 2).with_vgap(6).with_hgap(8).build();
    top.add_growable_col(1, 1);
    let r_name = put(&top, &dlg, "Name", name);
    let _ = put(&top, &dlg, "Database Type", dbtype);
    let _ = put(&top, &dlg, "Folder", folder);
    let ub = StaticBoxSizerBuilder::new_with_label(Orientation::Vertical, &dlg, "User Info").build();
    let ug = FlexGridSizer::builder(0, 2).with_vgap(6).with_hgap(8).build();
    ug.add_growable_col(1, 1);
    let rows_user = vec![put(&ug, &dlg, "Username", user), put(&ug, &dlg, "Password", pw), put(&ug, &dlg, "", save_pw), put(&ug, &dlg, "Role", role)];
    ub.add_sizer(&ug, 1, SizerFlag::Expand | SizerFlag::All, 6);
    let dbx = StaticBoxSizerBuilder::new_with_label(Orientation::Vertical, &dlg, "Details").build();
    let dg = FlexGridSizer::builder(0, 2).with_vgap(6).with_hgap(8).build();
    dg.add_growable_col(1, 1);
    let rows_det = vec![
        put(&dg, &dlg, "Connection Type", ctype),
        put(&dg, &dlg, "Hostname", host),
        put(&dg, &dlg, "Port", port),
        put_l(&dg, r_sid, sid),
        put_l(&dg, r_svc, svc),
        put(&dg, &dlg, "Database", db),
        put(&dg, &dlg, "Database file", file),
        put(&dg, &dlg, "", browse),
        put(&dg, &dlg, "TNS alias / Descriptor", tns),
        put(&dg, &dlg, "Options", opts),
    ];
    dbx.add_sizer(&dg, 1, SizerFlag::Expand | SizerFlag::All, 6);
    let _ = r_name;

    let load = move |c: &ConnConfig| {
        name.set_value(&c.name);
        dbtype.set_selection(ordered_drivers().iter().position(|d| d.id == c.driver).unwrap_or(0) as i32);
        folder.set_value(&c.folder);
        user.set_value(&c.user);
        pw.set_value(&if c.name.is_empty() { String::new() } else { config::get_password(&c.name).unwrap_or_default() });
        save_pw.set_value(c.save_password);
        role.set_selection(ROLES.iter().position(|r| *r == c.role).unwrap_or(0) as u32);
        ctype.set_selection(match c.conn_type.as_str() {
            "tns" => 1,
            "custom" => 2,
            _ => 0,
        });
        host.set_value(&c.host);
        port.set_value(&c.port.map(|p| p.to_string()).unwrap_or_default());
        r_sid.set_value(c.sid);
        r_svc.set_value(!c.sid);
        sid.set_value(if c.sid { &c.database } else { "" });
        svc.set_value(if c.sid { "" } else { &c.database });
        db.set_value(&c.database);
        file.set_value(&c.database);
        tns.set_value(&c.tns);
        opts.set_value(&c.options.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; "));
    };
    let rows_user = std::rc::Rc::new(rows_user);
    let rows_det = std::rc::Rc::new(rows_det);
    let sync = {
        let (ru, rd) = (rows_user.clone(), rows_det.clone());
        move || {
            let d = ordered_drivers()[dbtype.get_selection().max(0) as usize];
            let ora = d.backend == Kind::Oracle;
            let file_db = d.backend.is_file();
            let ct = ctype.get_selection().unwrap_or(0);
            let basic = !ora || ct == 0;
            for ((l, c), show) in ru.iter().zip([!file_db, !file_db, !file_db, ora]) {
                l.show(show);
                c.show(show);
            }
            let det = [ora, !file_db && basic, !file_db && basic, ora && ct == 0, ora && ct == 0, !ora && !file_db, file_db, file_db, ora && ct > 0, !file_db];
            for ((l, c), show) in rd.iter().zip(det) {
                l.show(show);
                c.show(show);
            }
            if port.get_value().is_empty() {
                if let Some(p) = d.port {
                    port.set_value(&p.to_string());
                }
            }
            dlg.layout();
        }
    };
    let init = existing.cloned().unwrap_or_else(blank);
    load(&init);
    sync();
    let orig: std::rc::Rc<std::cell::RefCell<Option<String>>> = std::rc::Rc::new(std::cell::RefCell::new(existing.map(|c| c.name.clone())));
    if let Some(i) = existing.and_then(|e| saved.iter().position(|c| c.name == e.name)) {
        list.set_item_state(i as i64, ListItemState::Selected, ListItemState::Selected);
    }
    let s2 = sync.clone();
    dbtype.on_selection_changed(move |_| {
        port.set_value("");
        s2()
    });
    let s3 = sync.clone();
    ctype.on_selection_changed(move |_| s3());
    let saved_v: Vec<ConnConfig> = saved.to_vec();
    let (o2, s4) = (orig.clone(), sync.clone());
    list.on_item_selected(move |e| {
        if let Some(c) = saved_v.get(e.get_item_index() as usize) {
            load(c);
            *o2.borrow_mut() = Some(c.name.clone());
            s4();
            status.set_label("Status :");
        }
    });
    browse.on_click(move |_| {
        let fd = FileDialog::builder(&dlg).with_message("Database file").with_style(FileDialogStyle::Open).build();
        if fd.show_modal() == ID_OK {
            if let Some(p) = fd.get_path() {
                file.set_value(&p);
            }
        }
    });

    let read = move || {
        let d = ordered_drivers()[dbtype.get_selection().max(0) as usize];
        let mut c = ConnConfig {
            name: name.get_value().trim().to_string(),
            driver: d.id.clone(),
            folder: folder.get_value().trim().to_string(),
            user: user.get_value().trim().to_string(),
            save_password: save_pw.get_value(),
            role: ROLES[role.get_selection().unwrap_or(0) as usize].into(),
            conn_type: ["basic", "tns", "custom"][ctype.get_selection().unwrap_or(0) as usize].into(),
            host: host.get_value().trim().to_string(),
            port: port.get_value().trim().parse().ok(),
            sid: d.backend == Kind::Oracle && r_sid.get_value(),
            tns: tns.get_value().trim().to_string(),
            options: opts.get_value().split(';').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect(),
            ..Default::default()
        };
        c.database = match d.backend {
            Kind::Oracle if c.sid => sid.get_value(),
            Kind::Oracle => svc.get_value(),
            k if k.is_file() => file.get_value(),
            _ => db.get_value(),
        }
        .trim()
        .to_string();
        if c.name.is_empty() {
            c.name = if d.backend.is_file() { c.database.rsplit(['/', '\\']).next().unwrap_or("db").to_string() } else { format!("{}@{}", c.user, c.host) };
        }
        c
    };

    let buttons = BoxSizer::builder(Orientation::Horizontal).build();
    let b_save = Button::builder(&dlg).with_label("Save").with_id(ID_SAVE).build();
    let b_clear = Button::builder(&dlg).with_label("Clear").build();
    let b_test = Button::builder(&dlg).with_label("Test").build();
    let b_connect = Button::builder(&dlg).with_label("Connect").with_id(ID_OK).build();
    let b_cancel = Button::builder(&dlg).with_label("Cancel").with_id(ID_CANCEL).build();
    buttons.add(&status, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
    for b in [&b_save, &b_clear, &b_test, &b_connect, &b_cancel] {
        buttons.add(b, 0, SizerFlag::All, 4);
    }
    let (o3, s5) = (orig.clone(), sync.clone());
    b_clear.on_click(move |_| {
        load(&blank());
        *o3.borrow_mut() = None;
        s5();
    });
    let test_conn: std::rc::Rc<std::cell::RefCell<Option<ConnRef>>> = Default::default();
    let tc = test_conn.clone();
    b_test.on_click(move |_| {
        status.set_label("Status : Testing…");
        let conn = Conn::open(read(), pw.get_value(), crate::app().prefs());
        *tc.borrow_mut() = Some(conn.clone());
        let tc2 = tc.clone();
        Conn::when_ready(&conn, move |r| {
            match r {
                Ok(()) => status.set_label("Status : Success"),
                Err(f) => {
                    status.set_label(&format!("Status : Failure - {}", f.error.lines().next().unwrap_or("")));
                    if let Some(d) = f.missing_driver.as_deref().and_then(registry::find) {
                        crate::driver_manager::download_with_progress(&dlg, d, move |ok| {
                            if ok {
                                status.set_label("Status : Driver installed, press Test again");
                            }
                        });
                    }
                }
            }
            tc2.borrow_mut().take();
        });
    });
    let taken: Vec<String> = saved.iter().map(|c| c.name.clone()).collect();
    let o4 = orig.clone();
    let validate = move |code: i32| {
        let c = read();
        if Some(&c.name) != o4.borrow().as_ref() && taken.contains(&c.name) {
            crate::ui::error(&dlg, &format!("A connection named \"{}\" already exists.", c.name));
            return;
        }
        dlg.end_modal(code);
    };
    let v2 = validate.clone();
    b_save.on_click(move |_| v2(ID_SAVE));
    b_connect.on_click(move |_| validate(ID_OK));

    let right = BoxSizer::builder(Orientation::Vertical).build();
    right.add_sizer(&top, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 18);
    right.add_sizer(&ub, 0, SizerFlag::Expand | SizerFlag::All, 6);
    right.add_sizer(&dbx, 1, SizerFlag::Expand | SizerFlag::All, 6);
    let body = BoxSizer::builder(Orientation::Horizontal).build();
    body.add(&list, 2, SizerFlag::Expand | SizerFlag::All, 8);
    body.add_sizer(&right, 3, SizerFlag::Expand, 0);
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_sizer(&body, 1, SizerFlag::Expand, 0);
    s.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 8);
    dlg.set_sizer(s, true);
    let code = dlg.show_modal();
    let (cfg, password) = (read(), pw.get_value());
    let old = orig.borrow().clone();
    dlg.destroy();
    (code == ID_OK || code == ID_SAVE).then_some((old, cfg, password, code == ID_OK))
}

fn put_l<L: WxWidget + 'static, W: WxWidget + 'static>(g: &FlexGridSizer, l: L, c: W) -> (Box<dyn WxWidget>, Box<dyn WxWidget>) {
    l.set_min_size(Size::new(LABEL_W, -1));
    g.add(&l, 0, SizerFlag::AlignCenterVertical, 0);
    g.add(&c, 1, SizerFlag::Expand, 0);
    (Box::new(l), Box::new(c))
}

/// Common databases first, then compatible engines grouped by family.
const POPULAR: [&str; 8] = ["oracle", "postgres", "mysql", "mariadb", "sqlserver", "sqlite", "duckdb", "clickhouse"];

fn ordered_drivers() -> Vec<&'static registry::Driver> {
    let all = registry::drivers();
    let mut v: Vec<&registry::Driver> = POPULAR.iter().filter_map(|id| registry::find(id)).collect();
    let mut rest: Vec<&registry::Driver> = all.iter().filter(|d| !POPULAR.contains(&d.id.as_str())).collect();
    rest.sort_by(|a, b| (&a.category, &a.label).cmp(&(&b.category, &b.label)));
    v.extend(rest);
    v
}

/// Same label column width in every section so fields line up across User Info / Details.
const LABEL_W: i32 = 150;

fn put<W: WxWidget + 'static>(g: &FlexGridSizer, dlg: &Dialog, label: &str, c: W) -> (Box<dyn WxWidget>, Box<dyn WxWidget>) {
    let l = StaticText::builder(dlg).with_label(label).build();
    l.set_min_size(Size::new(LABEL_W, -1));
    g.add(&l, 0, SizerFlag::AlignCenterVertical, 0);
    g.add(&c, 1, SizerFlag::Expand, 0);
    (Box::new(l), Box::new(c))
}
