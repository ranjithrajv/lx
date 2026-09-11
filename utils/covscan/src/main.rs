// SPDX-License-Identifier: GPL-3.0-or-later

use syn::visit::{self, Visit};
use syn::{ImplItemFn, ItemFn, ItemImpl, ItemMod};
use std::path::Path;

struct FnCollector {
    fns: Vec<(String, usize, usize)>,
    path_stack: Vec<String>,
    impl_stack: Vec<String>,
}
impl FnCollector {
    fn fqn(&self, name: &str) -> String {
        let mut s = String::new();
        for p in &self.path_stack {
            s.push_str(p);
            s.push_str("::");
        }
        for p in &self.impl_stack {
            s.push_str(p);
            s.push_str("::");
        }
        s.push_str(name);
        s
    }
}
impl<'ast> Visit<'ast> for FnCollector {
    fn visit_item_fn(&mut self, f: &'ast ItemFn) {
        let name = f.sig.ident.to_string();
        let start = f.sig.ident.span().start().line;
        let end = f.block.brace_token.span.close().end().line;
        self.fns.push((self.fqn(&name), start, end));
        visit::visit_item_fn(self, f);
    }
    fn visit_impl_item_fn(&mut self, m: &'ast ImplItemFn) {
        let name = m.sig.ident.to_string();
        let start = m.sig.ident.span().start().line;
        let end = m.block.brace_token.span.close().end().line;
        self.fns.push((self.fqn(&name), start, end));
        visit::visit_impl_item_fn(self, m);
    }
    fn visit_item_impl(&mut self, i: &'ast ItemImpl) {
        if let syn::Type::Path(tp) = &*i.self_ty {
            if let Some(seg) = tp.path.segments.last() {
                self.impl_stack.push(seg.ident.to_string());
            }
        }
        visit::visit_item_impl(self, i);
        self.impl_stack.pop();
    }
    fn visit_item_mod(&mut self, m: &'ast ItemMod) {
        if let Some((_, items)) = &m.content {
            self.path_stack.push(m.ident.to_string());
            for it in items {
                self.visit_item(it);
            }
            self.path_stack.pop();
        } else {
            visit::visit_item_mod(self, m);
        }
    }
}

fn scan(file: &Path) -> Vec<(String, usize, usize)> {
    let src = std::fs::read_to_string(file).unwrap();
    let ast = syn::parse_file(&src).unwrap();
    let mut c = FnCollector {
        fns: Vec::new(),
        path_stack: Vec::new(),
        impl_stack: Vec::new(),
    };
    for item in &ast.items {
        c.visit_item(item);
    }
    c.fns
}

fn main() {
    let dirs: Vec<String> = std::env::args().skip(1).collect();
    let mut out = serde_json::Map::new();
    for dir in dirs {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().map(|e| e == "rs").unwrap_or(false) {
                if p.file_name().unwrap().to_str().unwrap() == "main.rs" {
                    continue;
                }
                let fns = scan(&p);
                let name = p.file_name().unwrap().to_str().unwrap().to_string();
                let arr: Vec<serde_json::Value> = fns
                    .into_iter()
                    .map(|(n, s, e)| serde_json::json!([n, s, e]))
                    .collect();
                out.insert(name, serde_json::Value::Array(arr));
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&serde_json::Value::Object(out)).unwrap());
}