//! Planning the lines of the layout with a VPC: the shapes it is read by, before the search.

use crate::layout::{Layout, Rect};
use crate::lines::{gather, Line};
use crate::measure::{label_area, Measure};
use crate::model::{Model, Node};
use crate::path::{center, draft, facing, opposite, Draft, Scene, Shape};
use crate::route::{draw, Ctx, Side, Wire, RANK_MAIN, RANK_SIDE};
use crate::rules::{Rules, Scope};

/// With two AZs, a line to the other AZ's part turns this far below the source's top, by source AZ.
const MESH_CHANNELS: [i32; 2] = [110, 155];

const NAT: &str = "nat";
pub(crate) const IGW: &str = "igw";

const RANK_NAT: u8 = 1;

pub fn route(model: &Model, layout: &Layout, rules: &Rules) -> (Vec<Wire>, Vec<String>) {
    let ctx = Ctx { model, layout, rules, scene: Scene::new(layout, Measure::Grid), measure: Measure::Grid };
    let (specs, notes) = gather(model, layout, rules);
    let drafts = ctx.plan(&specs);
    draw(&ctx, &specs, &drafts, &[], notes)
}

impl<'a> Ctx<'a> {
    /// The AZ suffix of the subnet an icon sits in, e.g. "1a" for "pri-1a".
    fn subnet_suffix(&self, id: &str) -> Option<&'a str> {
        let p = self.icon(id).parent.as_deref()?;
        p.strip_prefix("pri-").or_else(|| p.strip_prefix("pub-"))
    }

    /// Sides and shape for every line.
    fn plan(&self, specs: &[Line]) -> Vec<Draft> {
        let mut drafts: Vec<Option<Draft>> = vec![None; specs.len()];
        let (mut nat_in, mut nat_out, mut side, mut rest) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (i, s) in specs.iter().enumerate() {
            let (f, t) = (self.node(&s.from), self.node(&s.to));
            let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
            let per_az = f.az.is_some() && t.az.is_some();
            drafts[i] = if self.scope(t) == Scope::Regional {
                if self.scope(f) == Scope::Vpc {
                    side.push(i);
                    None
                } else {
                    Some(self.above_to_side(t, tr))
                }
            } else if per_az && f.service == t.service {
                let h = facing(fr, tr);
                Some(self.first_fit(s, vec![
                    draft(h, opposite(h), Shape::Straight, RANK_SIDE),
                    draft(Side::Bottom, Side::Bottom, Shape::Search, RANK_SIDE),
                ]))
            } else if t.service == NAT && f.az.is_some() && f.az == t.az {
                nat_in.push(i);
                None
            } else if f.service == NAT && t.service == IGW {
                nat_out.push(i);
                None
            } else if let Some(d) = self
                .fan_out_channel(f, t, fr, tr)
                .map(|y| draft(Side::Bottom, Side::Top, Shape::ChannelY(y), RANK_MAIN))
                .filter(|d| self.fits(s, d))
            {
                Some(d)
            } else if per_az && self.model.azs.len() == 2 && tr.y > fr.y + fr.h {
                Some(self.mesh(f, t, fr))
            } else if let Some(d) = Some(draft(Side::Bottom, Side::Top, Shape::Straight, RANK_MAIN))
                .filter(|d| center(fr).0 == center(tr).0 && self.fits(s, d))
            {
                Some(d)
            } else {
                rest.push(i);
                None
            };
        }
        self.choose_together(specs, &nat_in, &mut drafts, |i| self.nat_in_variants(&specs[i]));
        self.choose_together(specs, &nat_out, &mut drafts, |i| self.nat_out_variants(&specs[i]));

        // Lines from the VPC to the side column jog in the gap between them, each in its own lane,
        // spread evenly between the VPC's side and the nearest icon or label in the column.
        side.sort_by_key(|&i| (self.layout.rect(&specs[i].to).y, i));
        let vpc = self.layout.rect("vpc");
        let n = side.len() as i32;
        let column = side
            .iter()
            .map(|&i| {
                let icon = self.icon(&specs[i].to);
                icon.rect.x.min(label_area(icon).x)
            })
            .min()
            .unwrap_or(vpc.x + vpc.w);
        for (k, &i) in side.iter().enumerate() {
            let x = vpc.x + vpc.w + (column - vpc.x - vpc.w) * (k as i32 + 1) / (n + 1);
            drafts[i] = Some(draft(Side::Right, Side::Left, Shape::ChannelX(x), RANK_SIDE));
        }

        self.plan_rest(specs, &rest, &mut drafts);
        drafts.into_iter().map(|d| d.expect("every line planned")).collect()
    }

    /// From above the Region (CloudFront) into a Regional service: down onto its top if nothing
    /// sits above it in the side column, else around the column's right and in from that side.
    fn above_to_side(&self, t: &Node, tr: Rect) -> Draft {
        let blocked = self.layout.icons.iter().any(|i| {
            i.id != t.id && i.rect.x < tr.x + tr.w && i.rect.x + i.rect.w > tr.x && i.rect.y < tr.y
        });
        if blocked {
            let region = self.layout.rect("region");
            let x = (tr.x + tr.w + region.x + region.w) / 2;
            draft(Side::Right, Side::Right, Shape::ChannelX(x), RANK_SIDE)
        } else {
            draft(Side::Right, Side::Top, Shape::Elbow, RANK_SIDE)
        }
    }

    /// From one part into each AZ's instance in a private subnet: along the gap between the
    /// target AZ's public and private subnets.
    fn fan_out_channel(&self, f: &Node, t: &Node, fr: Rect, tr: Rect) -> Option<i32> {
        if f.az.is_some() || t.az.is_none() || tr.y <= fr.y + fr.h {
            return None;
        }
        let p = self.icon(&t.id).parent.as_deref()?;
        let sfx = p.strip_prefix("pri-")?;
        let (public, private) = (self.layout.rect(&format!("pub-{sfx}")), self.layout.rect(p));
        Some((public.y + public.h + private.y) / 2)
    }

    /// With two AZs, every per-AZ part reaches every per-AZ part below it. The line inside an AZ
    /// drops straight on the outer quarter; the line to the other AZ turns in its own channel and
    /// enters the middle, so the two crossing lines never share a column.
    fn mesh(&self, f: &Node, t: &Node, fr: Rect) -> Draft {
        let az = f.az.expect("per-AZ").min(1);
        let mut d = draft(Side::Bottom, Side::Top, Shape::Straight, RANK_MAIN);
        if f.az == t.az {
            d.entry_pin = Some(if az == 0 { 25 } else { 75 });
        } else {
            d.entry_pin = Some(50);
            d.shape = Shape::ChannelY(fr.y + MESH_CHANNELS[az]);
        }
        d
    }

    /// Into the NAT gateway of the same AZ: sideways toward it and up into its bottom, or, if
    /// that passes over a frame name, up the gap between the AZ and subnet frames into its far side.
    fn nat_in_variants(&self, s: &Line) -> Vec<Draft> {
        let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
        let h = facing(fr, tr);
        let mut v = vec![draft(h, Side::Bottom, Shape::Elbow, RANK_NAT)];
        if let (Some(sfx), Some(sub)) = (self.subnet_suffix(&s.from), self.icon(&s.from).parent.as_deref()) {
            let (az, sub) = (self.layout.rect(&format!("az-{sfx}")), self.layout.rect(sub));
            let x = match h {
                Side::Left => (az.x + sub.x) / 2,
                _ => (az.x + az.w + sub.x + sub.w) / 2,
            };
            v.push(draft(h, h, Shape::ChannelX(x), RANK_NAT));
        }
        v
    }

    /// From a NAT gateway to the Internet Gateway: up from its top and into the gateway's side,
    /// or, if that passes over a frame name, out its far side and up the gap between the VPC and AZ frames.
    fn nat_out_variants(&self, s: &Line) -> Vec<Draft> {
        let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
        let h = facing(tr, fr);
        let mut v = vec![draft(Side::Top, h, Shape::Elbow, RANK_NAT)];
        if let Some(sfx) = self.subnet_suffix(&s.from) {
            let (az, vpc) = (self.layout.rect(&format!("az-{sfx}")), self.layout.rect("vpc"));
            let x = match h {
                Side::Left => (vpc.x + az.x) / 2,
                _ => (vpc.x + vpc.w + az.x + az.w) / 2,
            };
            v.push(draft(h, h, Shape::ChannelX(x), RANK_NAT));
        }
        v
    }

    /// Gives every member the first variant that fits for all of them, so AZs that mirror each
    /// other get mirrored lines; members no common variant fits get their own first fit.
    fn choose_together(&self, specs: &[Line], members: &[usize], drafts: &mut [Option<Draft>], variants: impl Fn(usize) -> Vec<Draft>) {
        let all: Vec<Vec<Draft>> = members.iter().map(|&i| variants(i)).collect();
        let common = (0..all.iter().map(Vec::len).min().unwrap_or(0))
            .find(|&v| members.iter().zip(&all).all(|(&i, vs)| self.fits(&specs[i], &vs[v])));
        for (&i, vs) in members.iter().zip(all) {
            drafts[i] = Some(match common {
                Some(v) => vs[v].clone(),
                None => self.first_fit(&specs[i], vs),
            });
        }
    }
}
