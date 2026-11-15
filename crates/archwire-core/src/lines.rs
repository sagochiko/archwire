//! Gathering semantic edges into the lines to draw, and the parts of each line's label.

use std::cmp::Reverse;

use crate::def::Rel;
use crate::model::Model;
use crate::route::Ctx;
use crate::rules::Scope;

/// Side lines drawn as one in the layout without a VPC: two or more auxiliary or association lines
/// into the same part, of the same kind, with the same label, from parts of the same service.
/// Indexes into `Model.edges`, in model order; one list per drawn line.
pub(crate) fn side_bundles(model: &Model) -> Vec<Vec<usize>> {
    let service = |id: &str| model.nodes.iter().find(|n| n.id == id).map(|n| n.service.as_str());
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, e) in model.edges.iter().enumerate().filter(|(_, e)| e.rel != Rel::Main && e.hole.is_none()) {
        let same = |k: usize| {
            let f = &model.edges[k];
            f.to == e.to && f.rel == e.rel && f.label == e.label && service(&f.from) == service(&e.from)
        };
        match out.iter_mut().find(|g| same(g[0])) {
            Some(g) => g.push(i),
            None => out.push(vec![i]),
        }
    }
    out.retain(|g| g.len() > 1);
    out
}

/// One line to draw and the semantic edges it stands for.
pub(crate) struct Spec {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) rel: Rel,
    pub(crate) hole: bool,
    /// Through a VPC endpoint: the label says so.
    endpoint: bool,
    label_key: String,
    /// Service of the merged sources, for lines that merge edges.
    merge: Option<String>,
    /// Indexes into `Model.edges`, in model order.
    pub(crate) edges: Vec<usize>,
    several_azs: bool,
    /// How many parts' side lines this one line stands for, when it stands for two or more.
    sources: usize,
}

impl<'a> Ctx<'a> {
    /// Which lines to draw. Lines into a Regional service from every AZ merge into one from the
    /// last AZ, nearest the side column. With three AZs, lines between per-AZ parts merge into the
    /// one inside the target's AZ. Everything else is one line per edge.
    pub(crate) fn group(&self) -> (Vec<Spec>, bool) {
        let mut specs: Vec<Spec> = Vec::new();
        for (i, e) in self.model.edges.iter().enumerate() {
            let (f, t) = (self.node(&e.from), self.node(&e.to));
            let merge = f.az.is_some()
                && (self.scope(t) == Scope::Regional
                    || (self.model.azs.len() > 2 && t.az.is_some() && f.service != t.service));
            let hole = e.hole.is_some();
            let same = |s: &Spec| {
                s.merge.as_deref() == Some(f.service.as_str()) && s.to == e.to && s.rel == e.rel && s.hole == hole && s.endpoint == e.endpoint
            };
            match specs.iter_mut().find(|s| merge && same(s)) {
                Some(s) => s.edges.push(i),
                None => specs.push(Spec {
                    from: e.from.clone(),
                    to: e.to.clone(),
                    rel: e.rel,
                    hole,
                    endpoint: e.endpoint,
                    label_key: e.label.clone(),
                    merge: merge.then(|| f.service.clone()),
                    edges: vec![i],
                    several_azs: false,
                    sources: 0,
                }),
            }
        }
        let mut merged_across_azs = false;
        for s in specs.iter_mut().filter(|s| s.edges.len() > 1) {
            let from = |e: usize| self.node(&self.model.edges[e].from);
            let to = self.node(&s.to);
            let rep = if self.scope(to) == Scope::Regional {
                *s.edges.iter().max_by_key(|&&e| (from(e).az, Reverse(e))).expect("not empty")
            } else {
                merged_across_azs = true;
                *s.edges.iter().find(|&&e| from(e).az == to.az).unwrap_or(&s.edges[0])
            };
            s.from = self.model.edges[rep].from.clone();
            let mut azs: Vec<Option<usize>> = s.edges.iter().map(|&e| from(e).az).collect();
            azs.sort_unstable();
            azs.dedup();
            s.several_azs = azs.len() > 1;
        }
        (specs, merged_across_azs)
    }

    /// Side lines gathered into one part become one line, from the sender nearest that part
    /// (the least vertical distance, then the first in the model); the note names the parts it stands for.
    pub(crate) fn bundle_side_lines(&self, specs: &mut Vec<Spec>, notes: &mut Vec<String>) {
        let (model, lang) = (self.model, self.model.lang);
        let icon = |id: &str| self.layout.icons.iter().find(|i| i.id == id).map(|i| i.rect);
        for bundle in side_bundles(model) {
            let edge = |e: usize| &model.edges[e];
            let Some(to) = icon(&edge(bundle[0]).to) else { continue };
            if bundle.iter().any(|&e| icon(&edge(e).from).is_none()) {
                continue;
            }
            let distance = |e: usize| (icon(&edge(e).from).expect("checked").y - to.y).abs();
            let rep = *bundle.iter().min_by_key(|&&e| (distance(e), e)).expect("a bundle has two or more lines");
            let at: Vec<usize> = bundle.iter().map(|&e| specs.iter().position(|s| s.edges.contains(&e)).expect("every edge has a line")).collect();
            let keep = at[0];
            specs[keep].from = edge(rep).from.clone();
            specs[keep].edges = bundle.clone();
            specs[keep].sources = bundle.len();
            let mut gone: Vec<usize> = at[1..].to_vec();
            gone.sort_unstable();
            for i in gone.into_iter().rev() {
                specs.remove(i);
            }
            let parts: Vec<&str> = bundle.iter().map(|&e| edge(e).from.as_str()).collect();
            notes.push(
                self.rules
                    .text("note.merged_side_lines", lang)
                    .replace("{from}", &edge(rep).from)
                    .replace("{to}", &edge(rep).to)
                    .replace("{parts}", &parts.join(&self.rules.text("parts_separator", lang))),
            );
        }
    }

    /// The label's parts: they read on one line joined by a space, or one per line where one line does not fit.
    pub(crate) fn label_parts(&self, s: &Spec) -> Vec<String> {
        let lang = self.model.lang;
        if s.hole {
            return vec![self.rules.text("undecided_route", lang)];
        }
        let mut parts = vec![self.rules.text(&s.label_key, lang)];
        if s.endpoint {
            parts.push(self.rules.text("via_endpoint", lang));
        }
        if s.several_azs {
            parts.push(self.rules.text("from_all_azs", lang));
        }
        if s.sources > 1 {
            parts.push(self.rules.text("from_sources", lang).replace("{n}", &s.sources.to_string()));
        }
        parts.retain(|p| !p.is_empty());
        parts
    }
}
