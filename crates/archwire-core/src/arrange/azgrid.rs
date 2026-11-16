//! The inside of a VPC: the Availability Zones side by side, the subnet tiers from top to bottom
//! in each, and the drafts of the lines inside the VPC and of the inner part of the lines that
//! cross its sides.
//!
//! The inside is laid out the same whichever way the outside runs; only the side the entry
//! gateway sits on follows the outside axis. Every distance comes from the parts' icons and names,
//! the frames' names and how many lines share a band, with the named values below as the least.
//! Four steps: the places of the parts across (columns), the lines and where they meet their
//! parts' sides, the places down (rows and bands, sized by the lines that run in them), and the
//! points of every line. Ties go to model order.

use std::collections::{BTreeMap, VecDeque};

use super::layered::ROW_STEP;
use super::check::failed;
use super::{Crossing, Lane};
use crate::diag::Problem;
use crate::ids::edge_id;
use crate::def::Rel;
use crate::layout::{icon, Frame, FrameKind, Icon, Layout, Rect};
use crate::lines::gather;
use crate::measure::{frame_name_box, name_area};
use crate::model::{Area, Axis, Model, Tier};
use crate::path::{grow, hits};
use crate::route::{port_point, Bend, Keep, LinePlan, Port, Side, Through, Track, LABEL_FIRST, LABEL_LINE};
use crate::rules::Rules;

type Pt = (i32, i32);

const ICON: i32 = 60;
pub(super) const VPC_FRAME: &str = "vpc";
const VPC_NAME: &str = "VPC";
/// From the VPC frame's sides to the zones, and from the zones to its bottom.
const VPC_SIDE: i32 = 30;
const VPC_TAIL: i32 = 20;
/// The least gap between two zones.
pub(super) const AZ_GAP: i32 = 20;
/// From a zone's top to its first subnet or band, past the zone's name, and from its last subnet to its bottom.
const AZ_HEAD: i32 = 30;
const AZ_TAIL: i32 = 15;
/// A subnet sits this far inside its zone on the left and right.
const SUB_INSET: i32 = 20;
/// From a subnet's top to its first icons, past the subnet's name and the lines coming in; from
/// its last names or band to its bottom; the least gap between two subnets; a subnet with nothing in it.
const SUB_HEAD: i32 = 50;
const SUB_TAIL: i32 = 20;
const SUB_GAP: i32 = 20;
const EMPTY_SUB: i32 = 60;
/// Names stay this far inside their subnet's sides.
pub(super) const PAD: i32 = 20;
/// A line keeps this far from icons, names and frame names, and from the lines beside it.
pub(super) const ROOM: i32 = 8;
/// The distance between two tracks in a band: a line's label and some room.
pub(super) const TRACK_STEP: i32 = 25;
/// The distance between two tracks, and from a track to the band's sides, in a band where the
/// layout keeps no room for a label: room, and the line's own width. The router may still put a
/// label on such a track; its last check reports a label it cannot keep clear.
pub(super) const TRACK_EDGE: i32 = ROOM + 2;
/// The least distance between the tops of two rows in a subnet, and from a band's last track to the row below.
const ROW_MIN: i32 = 210;
const ENTRY_ROOM: i32 = 50;
/// From the names of a row above the zones to what comes next.
const ROW_GAP: i32 = 40;
/// The least distance between the centres of two columns in a zone.
const COL_MIN: i32 = 200;
const NAME_ROOM: i32 = 20;
/// The least distance a public part (a NAT gateway) sits right of a zone's first column.
pub(super) const NAT_SHIFT: i32 = 120;
/// Lines running right: from the entry gateway's column to the zones.
const IGW_GAP: i32 = 40;
const FRAME_ROOM: i32 = 20;
/// How many times the widths are widened to keep lines clear before the layout is taken as it is.
const TRIES: usize = 1000;

/// The inside of one VPC, in the VPC frame's own coordinates.
pub(crate) struct Placed {
    pub w: i32,
    pub h: i32,
    pub icons: Vec<Icon>,
    /// The VPC frame first, then each zone's frame and its subnet frames.
    pub frames: Vec<Frame>,
    /// One draft per semantic edge with both ends inside, kept whole and running down.
    pub inside: Vec<LinePlan>,
    /// One per semantic edge that crosses the VPC frame's side.
    pub crossing: Vec<Crossing>,
    pub lanes: Vec<Lane>,
    /// The semantic edges whose drawn line runs along a band, decided apart from the tracks
    /// given out: a line that bends once or runs straight is not among them.
    pub needs: Vec<usize>,
}

/// What a part inside the VPC is to the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// On the side traffic from outside comes in by: the Internet Gateway.
    Entry,
    /// In a subnet tier, spanning the zones: a load balancer.
    Span,
    /// One copy per zone in the public tier: a NAT gateway.
    Public,
    /// One copy per zone in the private tier.
    Private,
}

/// A part of the definition inside the VPC: its copies, one per zone, or its one node.
struct Group {
    part: String,
    place: Place,
    /// Indexes into `Model.nodes`, in model order.
    nodes: Vec<usize>,
    /// The row in the private tier, and the column in its row or tier.
    row: usize,
    col: usize,
    /// Half the width its icon and name take, and how far below the icon's top its name ends.
    half: i32,
    name: i32,
}

/// How a line runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Straight between two sides facing each other; both ends at one position.
    Direct,
    /// Out of one side, along a track, into the facing side of the other end: down or up.
    Track,
    /// Between two parts of one row: down from one, along a track below the row, up into the other.
    Below,
    /// From a part to the NAT gateway of its zone: two forms to choose from.
    NatIn,
    /// From a NAT gateway to the entry gateway.
    NatOut,
    /// Out of the top or bottom, one corner, into a side.
    Elbow,
    /// From outside, through a gate straight onto the end.
    InGate,
    /// From outside, through a gate on the left side and along a track of the band above the tier.
    InTrack,
    /// To outside, out of the right side straight to a gate on the VPC's right side.
    Out,
    /// To outside, down to a track below the row, then right to a gate on the VPC's right side.
    OutTrack,
    /// To outside from a part spanning the zones whose row holds public copies right of it: down
    /// to a track of the band above the private tier, then right to a gate on the VPC's right side.
    OutFan,
    /// From a part to a public copy in another zone: out of the side facing that zone, to the gap
    /// between the zones, up to a track of the band above the private tier, along it, and up
    /// into the copy.
    NatAcross,
}

/// A band kept for lines: above the zones for the NAT gateways' lines when lines run right, above
/// the private tier, and below each private row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Band {
    Nat,
    Fan,
    Below(usize),
}

/// One semantic edge with an end inside the VPC.
#[derive(Debug, Clone)]
struct Run {
    edge: usize,
    from: usize,
    to: usize,
    kind: Kind,
    exit: Side,
    entry: Side,
    /// Drawn after lines are gathered: the representative of its line.
    drawn: bool,
    /// The position both ends of a straight line keep.
    pin: Option<u8>,
    exit_pos: u8,
    entry_pos: u8,
    band: Option<Band>,
    /// The track's number in its band, from the top.
    track: Option<usize>,
}

/// A set of spots on the sides of parts: a shared gap (1, zone, column, row) or one side (0, node, side, 0).
type Slot = (usize, usize, usize, u8);
/// One line's end at a side: its order along the side, its edge, its index in the runs, whether it is the exit.
type End = (i64, usize, usize, bool);

/// The places across: in VPC coordinates, the zones' left sides follow from `left`, `w` and `gap`;
/// the columns are from a zone's left side.
#[derive(Debug, Clone)]
struct Across {
    left: i32,
    w: i32,
    gap: i32,
    cols: Vec<i32>,
    pubs: Vec<i32>,
    spans: Vec<i32>,
    entry: i32,
    vpc_w: i32,
}

impl Across {
    fn az_x(&self, k: usize) -> i32 {
        self.left + k as i32 * (self.w + self.gap)
    }
}

/// The places down, in VPC coordinates.
#[derive(Debug, Clone, Default)]
struct Down {
    nat_top: i32,
    entry_y: i32,
    span_y: i32,
    az_top: i32,
    public: Option<(i32, i32)>,
    pub_row: i32,
    fan: (i32, i32),
    private: Option<(i32, i32)>,
    rows: Vec<i32>,
    below: Vec<i32>,
    az_bottom: i32,
    h: i32,
}

/// Something that does not keep clear across: a line (its id) or a part, what it comes too near
/// (a part's or a frame's id), how much further from it it has to be, and what for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Clash {
    what: String,
    against: String,
    short: i32,
    room: &'static str,
}

/// What has to change for the lines to keep clear: wider zones, a wider gap between them, or the
/// columns moved right in their zones.
enum Fix {
    Wider,
    Gap,
    Shift,
}

struct Grid<'a> {
    model: &'a Model,
    rules: &'a Rules,
    axis: Axis,
    boundary: bool,
    groups: Vec<Group>,
    /// The group of each model node inside the VPC.
    group_of: Vec<Option<usize>>,
    public: bool,
    private: bool,
    rows: usize,
    /// Half the width of each private column.
    col_half: Vec<i32>,
    names_right: i32,
    entry_half: i32,
    reps: Vec<usize>,
}

/// The inside of the VPC, or LAYOUT_FAILED naming a line that no widening keeps clear of a part
/// or a frame's name it passes.
pub(crate) fn lay_vpc(model: &Model, rules: &Rules, parts: &[usize], axis: Axis) -> Result<Placed, Vec<Problem>> {
    let g = Grid::new(model, rules, parts, axis);
    // Across: widen until the lines that pass a part or a frame's name keep clear of it. A widening
    // that leaves the same line exactly as close to the same thing does not move them apart, so
    // widening cannot clear it; nor can it when the tries run out.
    let (mut extra, mut gap, mut shift) = (0, AZ_GAP, 0);
    let mut across = g.across(extra, gap, shift);
    let mut runs = g.runs(&across);
    let mut last: Option<Clash> = None;
    let mut tries = 0;
    while let Some((fix, clash)) = g.conflict(&across, &runs) {
        let stuck = last.as_ref().is_some_and(|l| l.what == clash.what && l.against == clash.against && clash.short == l.short);
        if stuck || tries == TRIES {
            return Err(vec![failed(&clash.what, format!("{} and {}", clash.what, clash.against), clash.room)]);
        }
        match fix {
            Fix::Wider => extra += 2,
            Fix::Gap => gap += 2,
            Fix::Shift => shift += 2,
        }
        last = Some(clash);
        tries += 1;
        #[cfg(test)]
        WIDENED.fetch_max(tries, std::sync::atomic::Ordering::Relaxed);
        across = g.across(extra, gap, shift);
        runs = g.runs(&across);
    }
    let down = g.down(&runs);
    Ok(g.finish(&across, &down, runs))
}

/// The most widenings any layout took, for the tests to see how far from the limit they stay.
#[cfg(test)]
pub(super) static WIDENED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl<'a> Grid<'a> {
    fn new(model: &'a Model, rules: &'a Rules, parts: &[usize], axis: Axis) -> Grid<'a> {
        let mut groups: Vec<Group> = Vec::new();
        let mut group_of = vec![None; model.nodes.len()];
        for &i in parts {
            let n = &model.nodes[i];
            let place = match (n.area, n.az) {
                (Area::VpcEdge, _) => Place::Entry,
                (Area::Subnet(_), None) => Place::Span,
                (Area::Subnet(Tier::Public), Some(_)) => Place::Public,
                (Area::Subnet(Tier::Private), Some(_)) => Place::Private,
                _ => continue,
            };
            let key = if n.az.is_some() { n.part.clone() } else { n.id.clone() };
            let probe = probe(model, rules, i);
            let (half, name) = ((name_area(&probe).w + 1) / 2, bottom(name_area(&probe)));
            let at = match groups.iter().position(|g| g.part == key && g.place == place && n.az.is_some()) {
                Some(at) => at,
                None => {
                    groups.push(Group { part: key, place, nodes: Vec::new(), row: 0, col: 0, half: ICON / 2, name: ICON });
                    groups.len() - 1
                }
            };
            let g = &mut groups[at];
            g.nodes.push(i);
            g.half = g.half.max(half);
            g.name = g.name.max(name);
            group_of[i] = Some(at);
        }
        let public = groups.iter().any(|g| g.nodes.iter().any(|&i| model.nodes[i].area == Area::Subnet(Tier::Public)));
        let private = groups.iter().any(|g| g.nodes.iter().any(|&i| model.nodes[i].area == Area::Subnet(Tier::Private)));

        // Rows of the private tier: the longest path of main lines between private parts. A part
        // a main line comes into from elsewhere takes the first row; a line back to a part reached
        // earlier from the users does not count.
        let order = reach_order(model);
        let first = |g: &Group| g.nodes.iter().map(|&i| order[i]).min().unwrap_or(usize::MAX);
        let index = |id: &str| model.nodes.iter().position(|n| n.id == id);
        let private_of = |i: usize| group_of[i].filter(|&k| groups[k].place == Place::Private);
        let mut entries = vec![false; groups.len()];
        let mut links: Vec<(usize, usize)> = Vec::new();
        for e in model.edges.iter().filter(|e| e.rel == Rel::Main) {
            let (Some(a), Some(b)) = (index(&e.from), index(&e.to)) else { continue };
            match (private_of(a), private_of(b)) {
                (None, Some(gb)) => entries[gb] = true,
                (Some(ga), Some(gb)) if ga != gb => links.push((ga, gb)),
                _ => {}
            }
        }
        links.retain(|&(a, b)| !entries[b] && first(&groups[b]) > first(&groups[a]));
        let mut row = vec![0usize; groups.len()];
        for _ in 0..groups.len() {
            for &(a, b) in &links {
                row[b] = row[b].max(row[a] + 1);
            }
        }
        let rows = groups.iter().enumerate().filter(|(_, g)| g.place == Place::Private).map(|(k, _)| row[k] + 1).max().unwrap_or(0);
        let mut col_half: Vec<i32> = Vec::new();
        for r in 0..rows {
            let in_row = groups.iter_mut().enumerate().filter(|(k, g)| g.place == Place::Private && row[*k] == r);
            for (c, (_, g)) in in_row.enumerate() {
                g.row = r;
                g.col = c;
                if col_half.len() <= c {
                    col_half.push(ICON / 2);
                }
                col_half[c] = col_half[c].max(g.half);
            }
        }
        for place in [Place::Public, Place::Span] {
            for (c, g) in groups.iter_mut().filter(|g| g.place == place).enumerate() {
                g.col = c;
            }
        }

        // The widest name a line coming down into the first column passes.
        let frame_w = |kind: FrameKind, label: String| frame_name_box(&Frame { id: String::new(), kind, label, rect: Rect { x: 0, y: 0, w: 0, h: 0 }, parent: None }).w;
        let mut names_right = model.azs.iter().map(|a| frame_w(FrameKind::Az, a.clone())).max().unwrap_or(0);
        if public {
            names_right = names_right.max(SUB_INSET + frame_w(FrameKind::PublicSubnet, rules.text("public_subnet", model.lang)));
        }
        if private {
            names_right = names_right.max(SUB_INSET + frame_w(FrameKind::PrivateSubnet, rules.text("private_subnet", model.lang)));
        }
        let entry_half = groups.iter().filter(|g| g.place == Place::Entry).map(|g| g.half).max().unwrap_or(ICON / 2);
        // With two zones, the parts spanning them sit on the boundary between the zones.
        let boundary = model.azs.len() == 2;
        let mut g = Grid { model, rules, axis, boundary, groups, group_of, public, private, rows, col_half, names_right, entry_half, reps: Vec::new() };
        g.reps = g.drawn_inside();
        g
    }

    fn zones(&self) -> usize {
        self.model.azs.len().max(1)
    }
    fn place(&self, i: usize) -> Option<Place> {
        self.group_of[i].map(|k| self.groups[k].place)
    }
    fn group(&self, i: usize) -> &Group {
        &self.groups[self.group_of[i].expect("a part inside the VPC")]
    }
    fn az(&self, i: usize) -> usize {
        self.model.nodes[i].az.unwrap_or(0)
    }
    fn spans(&self) -> impl Iterator<Item = &Group> {
        self.groups.iter().filter(|g| g.place == Place::Span)
    }
    /// Whether the parts spanning the zones get a row of their own above them.
    fn above(&self) -> bool {
        self.spans().next().is_some() && !self.boundary
    }
    fn index(&self, id: &str) -> usize {
        self.model.nodes.iter().position(|n| n.id == id).expect("edge ends are nodes")
    }

    /// The edges with both ends inside that are drawn once lines are gathered. Which copy stands
    /// for a merged line inside the VPC follows from the zones alone, so a rough layout of the
    /// inside is enough to tell.
    fn drawn_inside(&self) -> Vec<usize> {
        let mut icons = Vec::new();
        for (k, g) in self.groups.iter().enumerate() {
            for &i in &g.nodes {
                let (x, y) = (self.az(i) as i32 * 1000 + g.col as i32 * 200, k as i32 * 300);
                icons.push(probe_at(self.model, self.rules, i, Rect { x, y, w: ICON, h: ICON }));
            }
        }
        let layout = Layout { frames: Vec::new(), icons };
        gather(self.model, &layout, self.rules).0.iter().map(|l| l.rep).collect()
    }

    fn across(&self, extra: i32, gap: i32, shift: i32) -> Across {
        let n = self.zones() as i32;
        let pubs_g: Vec<&Group> = self.groups.iter().filter(|g| g.place == Place::Public).collect();
        let span_half = self.spans().map(|g| g.half).max().unwrap_or(0);
        let c0_half = self.col_half.first().copied().unwrap_or(ICON / 2);
        let col0 = (SUB_INSET + PAD + c0_half).max(self.names_right + ROOM + 1) + shift;
        let mut pubs: Vec<i32> = Vec::new();
        for (k, p) in pubs_g.iter().enumerate() {
            let at = match k {
                0 => col0 + NAT_SHIFT.max(p.half + ROOM + 1).max(if span_half > 0 { span_half + ROOM + 1 } else { 0 }),
                _ => pubs[k - 1] + COL_MIN.max(pubs_g[k - 1].half + p.half + NAME_ROOM),
            };
            pubs.push(at);
        }
        let mut cols = vec![col0];
        for j in 1..self.col_half.len() {
            let mut at = cols[j - 1] + COL_MIN.max(self.col_half[j - 1] + self.col_half[j] + NAME_ROOM);
            if j == 1 {
                if let (Some(&last), Some(p)) = (pubs.last(), pubs_g.last()) {
                    at = at.max(last + p.half.max(self.col_half[1]) + ROOM + 1);
                }
            }
            cols.push(at);
        }
        let right = cols
            .iter()
            .zip(self.col_half.iter().chain(std::iter::repeat(&(ICON / 2))))
            .map(|(c, h)| c + h)
            .chain(pubs.iter().zip(&pubs_g).map(|(c, p)| c + p.half))
            .max()
            .unwrap_or(col0 + ICON / 2);
        let w = (right + PAD + SUB_INSET).max(self.names_right + PAD) + extra;
        let has_entry = self.groups.iter().any(|g| g.place == Place::Entry);
        let left = if self.axis == Axis::Right && has_entry { FRAME_ROOM + 2 * self.entry_half + IGW_GAP } else { VPC_SIDE };
        let mut a = Across { left, w, gap, cols, pubs, spans: Vec::new(), entry: 0, vpc_w: 0 };
        let zones_end = a.az_x(n as usize - 1) + w;
        let base = if n % 2 == 0 { a.az_x(n as usize / 2) - gap / 2 } else { a.az_x(n as usize / 2) + a.cols[0] };
        let span_g: Vec<&Group> = self.spans().collect();
        for (k, s) in span_g.iter().enumerate() {
            let at = if k == 0 { base } else { a.spans[k - 1] + COL_MIN.max(span_g[k - 1].half + s.half + NAME_ROOM) };
            a.spans.push(at);
        }
        a.entry = match (self.axis, a.spans.first()) {
            (Axis::Right, _) => FRAME_ROOM + self.entry_half,
            (Axis::Down, Some(&s)) => s,
            (Axis::Down, None) => (left + zones_end) / 2,
        };
        let vpc_name = frame_name_box(&Frame { id: String::new(), kind: FrameKind::Vpc, label: VPC_NAME.into(), rect: Rect { x: 0, y: 0, w: 0, h: 0 }, parent: None }).w;
        a.vpc_w = (zones_end + VPC_SIDE)
            .max(span_g.iter().zip(&a.spans).map(|(g, x)| x + g.half + PAD).max().unwrap_or(0))
            .max(vpc_name + FRAME_ROOM);
        a
    }

    /// The centre of a part across, in VPC coordinates.
    fn cx(&self, a: &Across, i: usize) -> i32 {
        let g = self.group(i);
        match g.place {
            Place::Entry => a.entry,
            Place::Span => a.spans[g.col],
            Place::Public => a.az_x(self.az(i)) + a.pubs[g.col],
            Place::Private => a.az_x(self.az(i)) + a.cols[g.col],
        }
    }

    /// Top to bottom: the entry gateway's row when lines run down, the row above the zones, the
    /// public tier, the private rows.
    fn level(&self, i: usize) -> usize {
        let g = self.group(i);
        match g.place {
            Place::Entry if self.axis == Axis::Down => 0,
            Place::Entry => 1,
            Place::Span if self.boundary => 2,
            Place::Span => 1,
            Place::Public => 3,
            Place::Private => 4 + g.row,
        }
    }

    /// Whether a straight line out of the right side of `i`, along its row, would pass another part.
    fn blocked(&self, i: usize) -> bool {
        let g = self.group(i);
        let (k, at) = (self.az(i), (self.az(i), g.col));
        self.groups.iter().filter(|o| o.place == g.place && o.row == g.row).flat_map(|o| o.nodes.iter().map(move |&j| (j, o.col))).any(|(j, c)| {
            let z = self.az(j);
            j != i && (z > k || (z == k && c > at.1))
        })
    }

    /// Every semantic edge with an end inside, with the way it runs and its sides; the positions
    /// on the sides are shared out among the drawn lines.
    fn runs(&self, a: &Across) -> Vec<Run> {
        let m = self.model;
        let mut out = Vec::new();
        for (edge, e) in m.edges.iter().enumerate() {
            let (f, t) = (self.index(&e.from), self.index(&e.to));
            let (pf, pt) = (self.place(f), self.place(t));
            if pf.is_none() && pt.is_none() {
                continue;
            }
            let mut r = Run { edge, from: f, to: t, kind: Kind::Elbow, exit: Side::Bottom, entry: Side::Top, drawn: true, pin: None, exit_pos: 50, entry_pos: 50, band: None, track: None };
            let side_to = |from: usize, to: usize| if self.cx(a, to) >= self.cx(a, from) { (Side::Right, Side::Left) } else { (Side::Left, Side::Right) };
            match (pf, pt) {
                (Some(x), Some(y)) => {
                    r.drawn = self.reps.contains(&edge);
                    let same_col = self.az(f) == self.az(t) && self.group(f).col == self.group(t).col;
                    match (x, y) {
                        (Place::Entry, Place::Span) => {
                            (r.exit, r.entry) = if self.axis == Axis::Down { (Side::Bottom, Side::Top) } else { (Side::Right, Side::Left) };
                            r.kind = Kind::Direct;
                            r.pin = Some(50);
                        }
                        (Place::Span, Place::Private) if self.cx(a, f) == self.cx(a, t) => {
                            r.kind = Kind::Direct;
                            r.pin = Some(50);
                        }
                        (Place::Span, Place::Private) => {
                            r.kind = Kind::Track;
                            r.band = Some(Band::Fan);
                        }
                        (Place::Private, Place::Public) if self.az(f) == self.az(t) => {
                            (r.kind, r.exit, r.entry) = (Kind::NatIn, side_to(f, t).0, Side::Bottom);
                        }
                        (Place::Private, Place::Public) if !self.beside_toward(f, t) => {
                            (r.kind, r.exit, r.entry, r.band) = (Kind::NatAcross, side_to(f, t).0, Side::Bottom, Some(Band::Fan));
                        }
                        (Place::Private, Place::Public) => {
                            (r.kind, r.exit, r.entry, r.band) = (Kind::Track, Side::Top, Side::Bottom, Some(Band::Fan));
                        }
                        (Place::Public, Place::Entry) => {
                            r.kind = Kind::NatOut;
                            r.exit = Side::Top;
                            if self.axis == Axis::Down {
                                r.entry = if self.cx(a, f) < a.entry { Side::Left } else { Side::Right };
                            } else {
                                r.entry = Side::Top;
                                r.band = Some(Band::Nat);
                            }
                        }
                        (Place::Private, Place::Private) => {
                            let (rf, rt) = (self.group(f).row, self.group(t).row);
                            if rf != rt {
                                let down = rf < rt;
                                (r.exit, r.entry) = if down { (Side::Bottom, Side::Top) } else { (Side::Top, Side::Bottom) };
                                if same_col && !self.in_column_between(f, t) {
                                    r.kind = Kind::Direct;
                                } else {
                                    r.kind = Kind::Track;
                                    r.band = Some(Band::Below(rf.min(rt)));
                                }
                            } else if self.az(f).abs_diff(self.az(t)) == 1 && self.group(f).col == self.group(t).col && !self.between(f, t) {
                                r.kind = Kind::Direct;
                                r.pin = Some(50);
                                (r.exit, r.entry) = side_to(f, t);
                            } else {
                                (r.kind, r.exit, r.entry, r.band) = (Kind::Below, Side::Bottom, Side::Bottom, Some(Band::Below(rf)));
                            }
                        }
                        _ => {
                            let (lf, lt) = (self.level(f), self.level(t));
                            r.exit = if lf <= lt { Side::Bottom } else { Side::Top };
                            r.entry = side_to(f, t).1;
                        }
                    }
                }
                (None, Some(y)) => {
                    r.kind = Kind::InGate;
                    r.entry = if self.axis == Axis::Down { Side::Top } else { Side::Left };
                    if y == Place::Private {
                        r.entry = Side::Top;
                        if self.axis == Axis::Right {
                            (r.kind, r.band) = (Kind::InTrack, Some(Band::Fan));
                        }
                    }
                }
                (Some(x), None) => {
                    r.kind = Kind::Out;
                    r.exit = Side::Right;
                    if m.nodes[f].az.is_some() && super::layered::only_inside(m, t) {
                        // A part joined to the VPC alone sits right of it, so of a part's copies,
                        // the one nearest the right side stands for them.
                        r.drawn = self.group(f).nodes.iter().all(|&j| self.az(j) <= self.az(f));
                    }
                    if x == Place::Private && r.drawn && self.blocked(f) {
                        (r.kind, r.exit, r.band) = (Kind::OutTrack, Side::Bottom, Some(Band::Below(self.group(f).row)));
                    }
                    if x == Place::Span && self.shares_row() && self.public_right_of(a, f) {
                        (r.kind, r.exit, r.band) = (Kind::OutFan, Side::Bottom, Some(Band::Fan));
                    }
                }
                (None, None) => unreachable!("skipped above"),
            }
            out.push(r);
        }
        self.pin_straight(a, &mut out);
        self.share_out(a, &mut out);
        self.number_tracks(a, &mut out);
        out
    }

    /// Whether the parts spanning the zones sit in the public tier's row of copies: on the
    /// boundary between the zones, unless the entry gateway's line comes into them from the side,
    /// where it would run along that row.
    fn shares_row(&self) -> bool {
        // The entry gateway's line into a spanning part comes in from the side when lines run right.
        let into_span = self.model.edges.iter().any(|e| self.place(self.index(&e.from)) == Some(Place::Entry) && self.place(self.index(&e.to)) == Some(Place::Span));
        let from_side = self.axis == Axis::Right && into_span;
        self.spans().next().is_some() && !self.above() && self.groups.iter().any(|g| g.place == Place::Public) && !from_side
    }

    /// Whether a part sits in the column of two copies of one zone, in a row between theirs.
    fn in_column_between(&self, f: usize, t: usize) -> bool {
        let (gf, gt) = (self.group(f), self.group(t));
        let (lo, hi) = (gf.row.min(gt.row), gf.row.max(gt.row));
        self.groups.iter().any(|o| o.place == Place::Private && o.col == gf.col && lo < o.row && o.row < hi)
    }

    /// Whether a part of the same row sits between two copies in neighbouring zones.
    fn between(&self, f: usize, t: usize) -> bool {
        let (g, lo, hi) = (self.group(f), self.az(f).min(self.az(t)), self.az(f).max(self.az(t)));
        self.groups.iter().filter(|o| o.place == Place::Private && o.row == g.row).flat_map(|o| o.nodes.iter().map(move |&j| (j, o.col))).any(|(j, c)| {
            let z = self.az(j);
            (z == lo && c > g.col) || (z == hi && c < g.col)
        })
    }

    /// Whether a part of the same row sits beside `f` in its zone on the side facing the zone of `t`.
    fn beside_toward(&self, f: usize, t: usize) -> bool {
        let (g, z) = (self.group(f), self.az(f));
        let left = self.az(t) < z;
        self.groups.iter().filter(|o| o.place == Place::Private && o.row == g.row).flat_map(|o| o.nodes.iter().map(move |&j| (j, o.col))).any(|(j, c)| {
            self.az(j) == z && if left { c < g.col } else { c > g.col }
        })
    }

    /// The middle of the gap between the zone of `f` and its neighbour toward the zone of `t`.
    fn gap_beside(&self, a: &Across, f: usize, t: usize) -> i32 {
        let z = self.az(f);
        if self.az(t) < z { a.az_x(z) - a.gap / 2 } else { a.az_x(z) + a.w + a.gap / 2 }
    }

    /// Whether a public copy sits right of `f` across.
    fn public_right_of(&self, a: &Across, f: usize) -> bool {
        let x = self.cx(a, f);
        self.groups.iter().filter(|g| g.place == Place::Public).flat_map(|g| g.nodes.iter()).any(|&i| self.cx(a, i) > x)
    }

    /// Straight lines between copies one above the other keep one position at both ends: toward
    /// the outer quarter, away from where the other lines at those sides come from.
    fn pin_straight(&self, a: &Across, runs: &mut [Run]) {
        // Where a line at a side goes; a line to or from outside the VPC counts as straight.
        let far = |r: &Run, end_of: usize, own: i32| {
            let other = if r.from == end_of { r.to } else { r.from };
            if self.place(other).is_some() { self.cx(a, other) } else { own }
        };
        for k in 0..runs.len() {
            let r = &runs[k];
            if r.kind != Kind::Direct || r.pin.is_some() {
                continue;
            }
            let reverse = runs.iter().any(|o| o.kind == Kind::Direct && o.from == r.to && o.to == r.from);
            let pin = if reverse {
                if r.exit == Side::Bottom { 25 } else { 75 }
            } else {
                let own = self.cx(a, r.from);
                let mut score = 0;
                for o in runs.iter().filter(|o| o.drawn && o.edge != r.edge) {
                    let at = |id: usize, side: Side| (o.from == id && o.exit == side) || (o.to == id && o.entry == side);
                    let (upper, s_up, lower, s_low) = if r.exit == Side::Bottom { (r.from, Side::Bottom, r.to, Side::Top) } else { (r.to, Side::Bottom, r.from, Side::Top) };
                    for (id, side) in [(upper, s_up), (lower, s_low)] {
                        if at(id, side) {
                            score += (far(o, id, own) - own).signum();
                        }
                    }
                }
                match score.signum() {
                    1 => 25,
                    -1 => 75,
                    _ => 50,
                }
            };
            runs[k].pin = Some(pin);
        }
    }

    /// The order of a line's end along a side: by where the line goes next.
    fn key(&self, a: &Across, r: &Run, at_exit: bool) -> i64 {
        let (me, other, side) = if at_exit { (r.from, r.to, r.exit) } else { (r.to, r.from, r.entry) };
        let known = self.place(other).is_some();
        let other_x = || if known { self.cx(a, other) as i64 } else { 0 };
        match (r.kind, side) {
            (Kind::InTrack, _) => -1_000_000,
            (Kind::OutTrack | Kind::OutFan, _) => 1_000_000,
            (Kind::NatIn, _) if at_exit => -1_000_000,
            (Kind::NatOut, _) if !at_exit => -((self.cx(a, r.from) - self.cx(a, r.to)).abs() as i64),
            (Kind::Out, _) => r.edge as i64,
            (Kind::InGate, _) => self.cx(a, me) as i64,
            (_, Side::Top | Side::Bottom) => other_x(),
            _ => (if known { self.level(other) as i64 } else { 0 }) * 1000 + r.edge as i64,
        }
    }

    /// Shares each side's positions out among the drawn lines' ends at it, as the router does:
    /// pinned ends first, then the rest in order.
    fn share_out(&self, a: &Across, runs: &mut [Run]) {
        // The bottom of a private copy and the top of the one below it in its column share one
        // set of spots: the lines leaving the one and entering the other run down the same gap.
        let slot = |i: usize, side: Side| -> Slot {
            let g = self.group(i);
            match (g.place, side) {
                (Place::Private, Side::Bottom) => (1, self.az(i), g.col, g.row as u8),
                (Place::Private, Side::Top) if g.row > 0 => (1, self.az(i), g.col, g.row as u8 - 1),
                _ => (0, i, side as usize, 0),
            }
        };
        let mut sides: BTreeMap<Slot, Vec<End>> = BTreeMap::new();
        for (k, r) in runs.iter().enumerate() {
            let inner = |id: usize| self.place(id).is_some();
            if r.drawn && inner(r.from) {
                sides.entry(slot(r.from, r.exit)).or_default().push((self.key(a, r, true), r.edge, k, true));
            }
            // A straight line down a gap takes one spot there.
            let once = inner(r.from) && r.kind == Kind::Direct && slot(r.from, r.exit) == slot(r.to, r.entry);
            if r.drawn && inner(r.to) && !once {
                sides.entry(slot(r.to, r.entry)).or_default().push((self.key(a, r, false), r.edge, k, false));
            }
        }
        for r in runs.iter_mut() {
            let p = r.pin.unwrap_or(50);
            (r.exit_pos, r.entry_pos) = (p, p);
        }
        for ((shared, _, side, _), mut ends) in sides {
            ends.sort();
            let pinned = ends.iter().filter(|e| runs[e.2].pin.is_some()).count();
            // A line coming down into a part beside one straight line takes the middle, away
            // from where the straight line comes down.
            let into_top = shared == 0 && side == Side::Top as usize && pinned == 1 && ends.len() == 2;
            let mut free = spots(ends.len());
            for &(_, _, k, _) in &ends {
                if let Some(p) = runs[k].pin {
                    let at = free.remove(nearest(&free, p));
                    // In a shared gap a straight line takes the spot itself, so the spots stay evenly apart.
                    if shared == 1 {
                        (runs[k].exit_pos, runs[k].entry_pos) = (at, at);
                    }
                }
            }
            if into_top {
                free = vec![50];
            }
            let mut next = free.into_iter();
            for &(_, _, k, is_exit) in &ends {
                if runs[k].pin.is_some() {
                    continue;
                }
                let p = next.next().expect("a spot for every end");
                if is_exit { runs[k].exit_pos = p } else { runs[k].entry_pos = p }
            }
        }
    }

    /// Numbers the tracks of each band: the line running furthest along it takes the top one, so
    /// lines turning into one band do not cross on the way in.
    fn number_tracks(&self, a: &Across, runs: &mut [Run]) {
        let mut bands: BTreeMap<Band, Vec<(i32, usize, usize)>> = BTreeMap::new();
        let mut straight = Vec::new();
        for (k, r) in runs.iter().enumerate() {
            let Some(b) = r.band else { continue };
            if !r.drawn {
                continue;
            }
            let (x0, x1) = self.track_ends(a, r);
            if x0 == x1 {
                straight.push(k);
                continue;
            }
            bands.entry(b).or_default().push((-(x1 - x0).abs(), r.edge, k));
        }
        // A line running straight on needs no track.
        for k in straight {
            runs[k].band = None;
        }
        for (_, mut users) in bands {
            users.sort();
            for (n, &(_, _, k)) in users.iter().enumerate() {
                runs[k].track = Some(n);
            }
        }
    }

    /// Where a line's stretch along its track starts and ends across.
    fn track_ends(&self, a: &Across, r: &Run) -> (i32, i32) {
        let at = |id: usize, side: Side, pos: u8| {
            let left = self.cx(a, id) - ICON / 2;
            match side {
                Side::Left => left,
                Side::Right => left + ICON,
                Side::Top | Side::Bottom => left + ICON * pos as i32 / 100,
            }
        };
        let (from, to) = (|| at(r.from, r.exit, r.exit_pos), || at(r.to, r.entry, r.entry_pos));
        match r.kind {
            Kind::InTrack => (0, to()),
            Kind::OutTrack | Kind::OutFan => (from(), a.vpc_w),
            Kind::NatAcross => (self.gap_beside(a, r.from, r.to), to()),
            Kind::NatOut => (self.cx(a, r.from), to()),
            _ => (from(), to()),
        }
    }

    /// The first thing that keeps a line from passing clear across: a line passing a part above
    /// the zones (widen the zones), or a line coming down from a part spanning the zones that runs
    /// by a frame's name or side or a public part (widen the gap the line comes down in, or move
    /// the columns right).
    fn conflict(&self, a: &Across, runs: &[Run]) -> Option<(Fix, Clash)> {
        let n = self.zones();
        let id = |i: usize| self.model.nodes[i].id.clone();
        let mut items: Vec<(i32, i32, String)> = self.spans().zip(&a.spans).map(|(g, &x)| (x, g.half, id(g.nodes[0]))).collect();
        if self.axis == Axis::Down {
            if let Some(g) = self.groups.iter().find(|g| g.place == Place::Entry) {
                items.push((a.entry, self.entry_half, id(g.nodes[0])));
            }
        }
        // How much further a line at x has to be from a part centred at c, half as wide as h.
        let short = |x: i32, c: i32, h: i32| h + ROOM - (x - c).abs();
        let line = |r: &Run| edge_id(&self.model.edges[r.edge].from, &self.model.edges[r.edge].to);
        let clash = |r: &Run, against: String, by: i32| Clash { what: line(r), against, short: by, room: "the line passing over it" };
        // Parts spanning the zones and public copies sharing their row keep their names apart.
        if self.shares_row() {
            let spans: Vec<(i32, i32, String)> = self.spans().zip(&a.spans).map(|(g, &x)| (x, g.half, id(g.nodes[0]))).collect();
            let copies = self.groups.iter().filter(|g| g.place == Place::Public).flat_map(|g| g.nodes.iter().map(move |&i| (a.az_x(self.az(i)) + a.pubs[g.col], g.half, i)));
            for (x, h, i) in copies {
                if let Some((c, sh, s)) = spans.iter().find(|&&(c, sh, _)| (x - c).abs() < h + sh + NAME_ROOM) {
                    let by = h + sh + NAME_ROOM - (x - c).abs();
                    return Some((Fix::Wider, Clash { what: s.clone(), against: id(i), short: by, room: "its name apart from the part beside it" }));
                }
            }
        }
        for r in runs.iter().filter(|r| r.drawn) {
            let pass = match (r.kind, self.place(r.to)) {
                (Kind::NatOut, _) => Some(self.cx(a, r.from)),
                (Kind::InGate, Some(Place::Private)) if self.axis == Axis::Down => Some(self.cx(a, r.to)),
                _ => None,
            };
            let Some(x) = pass else { continue };
            if let Some((c, h, it)) = items.iter().find(|&&(c, h, _)| short(x, c, h) > 0) {
                return Some((Fix::Wider, clash(r, it.clone(), short(x, *c, *h))));
            }
        }
        let fix = || if n.is_multiple_of(2) { Fix::Gap } else { Fix::Shift };
        let pubs: Vec<(i32, i32, String)> = self
            .groups
            .iter()
            .filter(|g| g.place == Place::Public)
            .flat_map(|g| g.nodes.iter().map(move |&i| (a.az_x(self.az(i)) + a.pubs[g.col], g.half, i)))
            .map(|(x, h, i)| (x, h, id(i)))
            .collect();
        let frame_w = |kind: FrameKind, label: String| frame_name_box(&Frame { id: String::new(), kind, label, rect: Rect { x: 0, y: 0, w: 0, h: 0 }, parent: None }).w;
        let sfx = |k: usize| self.model.azs[k][self.model.azs[k].len() - 2..].to_string();
        for r in runs.iter().filter(|r| r.drawn) {
            let spanning = self.place(r.from) == Some(Place::Span) && self.place(r.to) == Some(Place::Private);
            let into_span = self.axis == Axis::Down && self.boundary && self.place(r.from) == Some(Place::Entry) && self.place(r.to) == Some(Place::Span);
            if !spanning && !into_span {
                continue;
            }
            let x = if into_span { a.entry } else { self.cx(a, r.from) - ICON / 2 + ICON * r.exit_pos as i32 / 100 };
            if let Some((c, h, it)) = pubs.iter().find(|&&(c, h, _)| short(x, c, h) > 0) {
                return Some((fix(), clash(r, it.clone(), short(x, *c, *h))));
            }
            for k in 0..n {
                let z = a.az_x(k);
                let mut borders = vec![(z, format!("az-{}", sfx(k))), (z + a.w, format!("az-{}", sfx(k)))];
                if self.public {
                    borders.extend([(z + SUB_INSET, format!("pub-{}", sfx(k))), (z + a.w - SUB_INSET, format!("pub-{}", sfx(k)))]);
                }
                if let Some((b, f)) = borders.iter().find(|&&(b, _)| (x - b).abs() < ROOM) {
                    return Some((fix(), clash(r, f.clone(), ROOM - (x - b).abs())));
                }
                if self.above() || into_span {
                    let mut names = vec![(z, z + frame_w(FrameKind::Az, self.model.azs[k].clone()), format!("az-{}", sfx(k)))];
                    if self.public {
                        let w = frame_w(FrameKind::PublicSubnet, self.rules.text("public_subnet", self.model.lang));
                        names.push((z + SUB_INSET, z + SUB_INSET + w, format!("pub-{}", sfx(k))));
                    }
                    if let Some((l, rt, f)) = names.iter().find(|&&(l, rt, _)| x > l - ROOM && x < rt + ROOM) {
                        return Some((fix(), clash(r, f.clone(), (x - (l - ROOM)).min(rt + ROOM - x))));
                    }
                }
            }
        }
        None
    }

    /// The places down, from the top: the band for the NAT gateways' lines when lines run right,
    /// the entry gateway's row when they run down, the row above the zones, then in every zone the
    /// public tier, the band above the private tier and the private rows, each with its band below.
    fn down(&self, runs: &[Run]) -> Down {
        let count = |b: Band| tracks_in(runs, b);
        let names = |p: Place| self.groups.iter().filter(|g| g.place == p).map(|g| g.name).max().unwrap_or(ICON);
        let mut d = Down::default();
        let mut y = self.head();
        let nats = count(Band::Nat);
        if nats > 0 {
            d.nat_top = y;
            y += (nats - 1) * TRACK_STEP + ROW_GAP;
        }
        let entry = self.groups.iter().any(|g| g.place == Place::Entry);
        if self.axis == Axis::Down && entry {
            d.entry_y = y;
            y += names(Place::Entry) + ROW_GAP;
        }
        let spans = self.spans().next().is_some();
        if self.above() {
            d.span_y = if self.axis == Axis::Down && entry { y.max(d.entry_y + ROW_STEP) } else { y };
            y = d.span_y + names(Place::Span) + ROW_GAP;
        }
        d.az_top = y;
        let mut cur = y + AZ_HEAD;
        let pub_parts = self.groups.iter().any(|g| g.place == Place::Public);
        if self.public {
            let top = cur;
            let mut last = top;
            if spans && !self.above() {
                d.span_y = top + SUB_HEAD;
                if self.axis == Axis::Down && entry {
                    d.span_y = d.span_y.max(d.entry_y + ROW_STEP);
                }
                last = d.span_y + names(Place::Span);
            }
            if pub_parts {
                d.pub_row = if self.shares_row() {
                    d.span_y
                } else if last == top {
                    top + SUB_HEAD
                } else {
                    last + ROW_GAP
                };
                last = last.max(d.pub_row + names(Place::Public));
            }
            let bottom = if last == top { top + EMPTY_SUB } else { last + SUB_TAIL };
            d.public = Some((top, bottom));
            cur = bottom;
        }
        if self.private {
            let fans = count(Band::Fan);
            let pitch = self.fan_pitch(runs);
            d.fan = (cur, cur + SUB_GAP.max(2 * pitch + (fans - 1).max(0) * pitch));
            let top = d.fan.1;
            let mut y = top + SUB_HEAD;
            let mut end = y;
            for r in 0..self.rows {
                let below_names = self.groups.iter().filter(|g| g.place == Place::Private && g.row == r).map(|g| g.name).max().unwrap_or(ICON);
                d.rows.push(y);
                d.below.push(y + below_names);
                let n = count(Band::Below(r));
                let band_end = y + below_names + if n > 0 { (n + 1) * TRACK_STEP } else { 0 };
                end = band_end;
                y = (y + ROW_MIN).max(band_end + ENTRY_ROOM);
            }
            let bottom = end + SUB_TAIL;
            d.private = Some((top, bottom));
            cur = bottom;
        }
        d.az_bottom = cur + AZ_TAIL;
        d.h = d.az_bottom + VPC_TAIL;
        if self.axis == Axis::Right {
            d.entry_y = if spans {
                d.span_y
            } else if pub_parts {
                d.pub_row
            } else if let Some(&r) = d.rows.first() {
                r
            } else {
                (d.az_top + d.az_bottom) / 2 - ICON / 2
            };
        }
        d
    }

    fn track_y(&self, d: &Down, runs: &[Run], r: &Run) -> Option<i32> {
        let k = r.track? as i32;
        Some(match r.band? {
            Band::Nat => d.nat_top + k * TRACK_STEP,
            Band::Fan => {
                let pitch = self.fan_pitch(runs);
                d.fan.0 + (d.fan.1 - d.fan.0 - (tracks_in(runs, Band::Fan) - 1) * pitch) / 2 + k * pitch
            }
            Band::Below(row) => d.below[row] + (k + 1) * TRACK_STEP,
        })
    }

    fn finish(&self, a: &Across, d: &Down, runs: Vec<Run>) -> Placed {
        let m = self.model;
        let lang = m.lang;
        let sfx = |k: usize| m.azs[k][m.azs[k].len() - 2..].to_string();
        // Icons, in model order.
        let mut icons: Vec<Icon> = Vec::new();
        for (i, g) in self.group_of.iter().enumerate() {
            let Some(k) = *g else { continue };
            let g = &self.groups[k];
            let (y, parent) = match g.place {
                Place::Entry => (d.entry_y, VPC_FRAME.to_string()),
                Place::Span => (d.span_y, VPC_FRAME.to_string()),
                Place::Public => (d.pub_row, format!("pub-{}", sfx(self.az(i)))),
                Place::Private => (d.rows[g.row], format!("pri-{}", sfx(self.az(i)))),
            };
            icons.push(probe_at(m, self.rules, i, Rect { x: self.cx(a, i) - ICON / 2, y, w: ICON, h: ICON }).with_parent(parent));
        }
        let frame = |id: String, kind: FrameKind, label: String, rect: Rect, parent: String| Frame { id, kind, label, rect, parent: Some(parent) };
        let mut frames = vec![Frame { id: VPC_FRAME.into(), kind: FrameKind::Vpc, label: VPC_NAME.into(), rect: Rect { x: 0, y: 0, w: a.vpc_w, h: d.h }, parent: Some(super::layered::REGION_FRAME.into()) }];
        for (k, zone) in m.azs.iter().enumerate() {
            let x = a.az_x(k);
            let az = format!("az-{}", sfx(k));
            frames.push(frame(az.clone(), FrameKind::Az, zone.clone(), Rect { x, y: d.az_top, w: a.w, h: d.az_bottom - d.az_top }, VPC_FRAME.into()));
            let sub = |top: i32, bottom: i32| Rect { x: x + SUB_INSET, y: top, w: a.w - 2 * SUB_INSET, h: bottom - top };
            if let Some((t, b)) = d.public {
                frames.push(frame(format!("pub-{}", sfx(k)), FrameKind::PublicSubnet, self.rules.text("public_subnet", lang), sub(t, b), az.clone()));
            }
            if let Some((t, b)) = d.private {
                frames.push(frame(format!("pri-{}", sfx(k)), FrameKind::PrivateSubnet, self.rules.text("private_subnet", lang), sub(t, b), az.clone()));
            }
        }

        let find = |i: usize| icons.iter().find(|ic| ic.id == m.nodes[i].id).expect("every part inside has an icon");
        let port = |i: usize, side: Side, pos: u8| {
            let ic = find(i);
            let offset = if side == Side::Bottom { LABEL_FIRST + LABEL_LINE * (ic.lines as i32 - 1) } else { 0 };
            port_point(ic.rect, Port { side, pos, offset })
        };
        // Every line's points, between its exit and its entry (inside) or between its inner end and
        // its gate (crossing), with its track. The forms of the lines into the NAT gateways are
        // chosen afterwards, the same for every zone where one form fits them all.
        struct Drawn {
            exit: Side,
            entry: Side,
            exit_pos: u8,
            entry_pos: u8,
            points: Vec<Pt>,
            gate: Option<Pt>,
            track: Option<i32>,
        }
        let mut drawn: Vec<Vec<Drawn>> = Vec::new();
        for r in &runs {
            let ty = self.track_y(d, &runs, r);
            let base = |points: Vec<Pt>, gate: Option<Pt>| Drawn { exit: r.exit, entry: r.entry, exit_pos: r.exit_pos, entry_pos: r.entry_pos, points, gate, track: ty };
            let inner_end = |i: usize| self.place(i).is_some();
            let (e, n) = (inner_end(r.from).then(|| port(r.from, r.exit, r.exit_pos)), inner_end(r.to).then(|| port(r.to, r.entry, r.entry_pos)));
            let forms = match r.kind {
                Kind::Direct => vec![base(Vec::new(), None)],
                Kind::Track | Kind::Below => {
                    let (e, n) = (e.expect("inside"), n.expect("inside"));
                    if e.0 == n.0 && r.kind == Kind::Track {
                        vec![base(Vec::new(), None)]
                    } else {
                        let y = ty.unwrap_or_else(|| self.middle_of(d, r));
                        vec![base(vec![(e.0, y), (n.0, y)], None)]
                    }
                }
                Kind::NatIn => {
                    let (e, n) = (e.expect("inside"), n.expect("inside"));
                    let x = a.az_x(self.az(r.from)) + if r.exit == Side::Right { a.w - SUB_INSET / 2 } else { SUB_INSET / 2 };
                    let nat = find(r.to).rect;
                    let side = r.exit;
                    let up = Drawn { exit: r.exit, entry: crate::path::opposite(side), exit_pos: r.exit_pos, entry_pos: 50, points: vec![(x, e.1), (x, nat.y + ICON / 2)], gate: None, track: None };
                    vec![base(vec![(n.0, e.1)], None), up]
                }
                Kind::NatOut => {
                    let (e, n) = (e.expect("inside"), n.expect("inside"));
                    match ty {
                        Some(y) => vec![base(vec![(e.0, y), (n.0, y)], None)],
                        None => vec![base(vec![(e.0, n.1)], None)],
                    }
                }
                Kind::Elbow => {
                    let (e, n) = (e.expect("inside"), n.expect("inside"));
                    vec![base(vec![(e.0, n.1)], None)]
                }
                Kind::InGate => {
                    let n = n.expect("inside");
                    let gate = if r.entry == Side::Top { (n.0, 0) } else { (0, n.1) };
                    vec![base(Vec::new(), Some(gate))]
                }
                Kind::InTrack => {
                    let n = n.expect("inside");
                    let y = ty.unwrap_or(n.1 - SUB_HEAD);
                    vec![base(vec![(n.0, y)], Some((0, y)))]
                }
                Kind::Out => {
                    let e = e.expect("inside");
                    vec![base(Vec::new(), Some((a.vpc_w, e.1)))]
                }
                Kind::OutTrack | Kind::OutFan => {
                    let e = e.expect("inside");
                    let y = ty.unwrap_or_else(|| self.middle_of(d, r));
                    vec![base(vec![(e.0, y)], Some((a.vpc_w, y)))]
                }
                Kind::NatAcross => {
                    let (e, n) = (e.expect("inside"), n.expect("inside"));
                    let x = self.gap_beside(a, r.from, r.to);
                    let y = ty.unwrap_or_else(|| self.middle_of(d, r));
                    vec![base(vec![(x, e.1), (x, y), (n.0, y)], None)]
                }
            };
            drawn.push(forms);
        }

        // Choose the forms: every drawn line with one form is taken first; then each set of lines
        // between the copies of two parts takes the first form that keeps clear in every zone.
        let path_of = |r: &Run, f: &Drawn| -> Vec<Pt> {
            let inner = |i: usize| self.place(i).is_some();
            let mut v = Vec::new();
            match f.gate {
                None => {
                    v.push(port(r.from, f.exit, f.exit_pos));
                    v.extend(f.points.iter().copied());
                    v.push(port(r.to, f.entry, f.entry_pos));
                }
                Some(gate) if inner(r.from) => {
                    v.push(port(r.from, f.exit, f.exit_pos));
                    v.extend(f.points.iter().copied());
                    v.push(gate);
                }
                Some(gate) => {
                    v.push(gate);
                    v.extend(f.points.iter().copied());
                    v.push(port(r.to, f.entry, f.entry_pos));
                }
            }
            v
        };
        let mut chosen: Vec<usize> = vec![0; runs.len()];
        let mut taken: Vec<Vec<Pt>> = Vec::new();
        for (k, r) in runs.iter().enumerate() {
            if r.drawn && drawn[k].len() == 1 {
                taken.push(path_of(r, &drawn[k][0]));
            }
        }
        let mut sets: Vec<(usize, usize, Vec<usize>)> = Vec::new();
        for (k, r) in runs.iter().enumerate().filter(|(k, r)| r.drawn && drawn[*k].len() > 1) {
            let key = (self.group_of[r.from].unwrap_or(usize::MAX), self.group_of[r.to].unwrap_or(usize::MAX));
            match sets.iter_mut().find(|s| (s.0, s.1) == key) {
                Some(s) => s.2.push(k),
                None => sets.push((key.0, key.1, vec![k])),
            }
        }
        for (_, _, members) in sets {
            let forms = members.iter().map(|&k| drawn[k].len()).min().unwrap_or(0);
            let fits = |k: usize, f: usize, taken: &[Vec<Pt>]| self.clear(&icons, &frames, &runs[k], &path_of(&runs[k], &drawn[k][f]), taken);
            let common = (0..forms).find(|&f| members.iter().all(|&k| fits(k, f, &taken)));
            for &k in &members {
                let f = common.or_else(|| (0..drawn[k].len()).find(|&f| fits(k, f, &taken))).unwrap_or(0);
                chosen[k] = f;
                taken.push(path_of(&runs[k], &drawn[k][f]));
            }
        }

        // The drafts, the gates and the bands.
        let mut inside = Vec::new();
        let mut crossing = Vec::new();
        let mut lanes: BTreeMap<Band, Vec<(usize, i32)>> = BTreeMap::new();
        for (k, r) in runs.iter().enumerate() {
            let f = &drawn[k][chosen[k]];
            let track = match (f.track, r.drawn) {
                (Some(y), true) => {
                    let pts = path_of(r, f);
                    let s = pts.windows(2).find(|s| s[0].1 == y && s[1].1 == y && s[0].0 != s[1].0);
                    s.map(|s| Track { horizontal: true, from: s[0].0.min(s[1].0), to: s[0].0.max(s[1].0), at: y })
                }
                _ => None,
            };
            if let (Some(t), Some(b)) = (track, r.band) {
                lanes.entry(b).or_default().push((r.edge, t.at));
            }
            let (from, to) = (m.nodes[r.from].id.clone(), m.nodes[r.to].id.clone());
            match f.gate {
                None => inside.push(LinePlan {
                    edge: r.edge,
                    from,
                    to,
                    exit: f.exit,
                    entry: f.entry,
                    bend: if f.points.is_empty() { Bend::Straight } else { Bend::Points(f.points.clone()) },
                    exit_pos: Some(f.exit_pos),
                    entry_pos: Some(f.entry_pos),
                    keep: Keep::Whole,
                    tracks: track.into_iter().collect(),
                    axis: Axis::Down,
                }),
                Some(gate) => {
                    let leaving = self.place(r.from).is_some();
                    let (inner_side, inner_pos) = if leaving { (f.exit, f.exit_pos) } else { (f.entry, f.entry_pos) };
                    crossing.push(Crossing { edge: r.edge, frame: VPC_FRAME.into(), through: Through { gate, inner: f.points.clone(), inner_is_start: leaving }, inner_side, inner_pos });
                }
            }
        }
        let lanes = lanes
            .into_iter()
            .map(|(b, tracks)| {
                let (lo, hi) = match b {
                    Band::Nat => (d.nat_top, d.nat_top + (tracks_in(&runs, b) - 1) * TRACK_STEP),
                    Band::Fan => d.fan,
                    Band::Below(row) => (d.below[row], d.below[row] + (tracks_in(&runs, b) + 1) * TRACK_STEP),
                };
                Lane { frame: VPC_FRAME.into(), horizontal: true, from: 0, to: a.vpc_w, lo, hi, tracks }
            })
            .collect();
        let needs = runs.iter().filter(|r| r.drawn && r.band.is_some()).map(|r| r.edge).collect();
        Placed { w: a.vpc_w, h: d.h, icons, frames, inside, crossing, lanes, needs }
    }

    /// From the VPC frame's top to the first thing in it: past the frame's name, with room.
    fn head(&self) -> i32 {
        let name = frame_name_box(&Frame { id: String::new(), kind: FrameKind::Vpc, label: VPC_NAME.into(), rect: Rect { x: 0, y: 0, w: 0, h: 0 }, parent: None });
        name.h + ROOM
    }

    /// How far apart the tracks of the band above the private tier are, and how far they keep
    /// from its sides (the subnet frames and the private subnet's name): a label's height when a
    /// line in it comes from outside along the outside axis, the line the layout keeps room for a
    /// label for; else room and the line's own width, with no room kept for a label.
    fn fan_pitch(&self, runs: &[Run]) -> i32 {
        let labelled = runs.iter().any(|r| r.band == Some(Band::Fan) && r.track.is_some() && r.kind == Kind::InTrack);
        if labelled { TRACK_STEP } else { TRACK_EDGE }
    }

    /// The middle of the band a line not drawn would use.
    fn middle_of(&self, d: &Down, r: &Run) -> i32 {
        match r.band {
            Some(Band::Fan) => (d.fan.0 + d.fan.1) / 2,
            Some(Band::Below(row)) => d.below[row] + TRACK_STEP / 2,
            _ => d.az_top,
        }
    }

    /// Whether a path keeps clear of every icon but its ends', every name but its ends', every
    /// frame's name, and of the paths already taken, which it may cross but not run along.
    fn clear(&self, icons: &[Icon], frames: &[Frame], r: &Run, path: &[Pt], taken: &[Vec<Pt>]) -> bool {
        let ends = [self.model.nodes[r.from].id.as_str(), self.model.nodes[r.to].id.as_str()];
        path.windows(2).all(|s| {
            let (a, b) = (s[0], s[1]);
            let things = icons.iter().filter(|i| !ends.contains(&i.id.as_str())).all(|i| !hits(a, b, grow(i.rect, ROOM)) && !hits(a, b, grow(name_area(i), ROOM)));
            let names = frames.iter().all(|f| !hits(a, b, grow(frame_name_box(f), ROOM)));
            let apart = taken.iter().flat_map(|p| p.windows(2)).all(|t| !along(a, b, t[0], t[1]));
            things && names && apart
        })
    }
}

/// Whether two segments run along each other, closer than `ROOM` and side by side for some length.
fn along(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let common = |p: i32, q: i32, r: i32, s: i32| p.max(q).min(r.max(s)) - p.min(q).max(r.min(s));
    if a.0 == b.0 && c.0 == d.0 {
        (a.0 - c.0).abs() < ROOM && common(a.1, b.1, c.1, d.1) > 0
    } else if a.1 == b.1 && c.1 == d.1 {
        (a.1 - c.1).abs() < ROOM && common(a.0, b.0, c.0, d.0) > 0
    } else {
        false
    }
}

/// The order the parts are first reached in from the users, along main lines, breadth first;
/// parts no main line from the users reaches come after, in model order.
fn reach_order(model: &Model) -> Vec<usize> {
    let n = model.nodes.len();
    let index = |id: &str| model.nodes.iter().position(|x| x.id == id);
    let mut order = vec![usize::MAX; n];
    let mut next = 0;
    let roots = (0..n).filter(|&i| model.nodes[i].area == Area::Outside).chain(0..n);
    for root in roots {
        if order[root] != usize::MAX {
            continue;
        }
        order[root] = next;
        next += 1;
        let mut queue = VecDeque::from([root]);
        while let Some(a) = queue.pop_front() {
            for e in model.edges.iter().filter(|e| e.rel == Rel::Main && e.from == model.nodes[a].id) {
                if let Some(b) = index(&e.to).filter(|&b| order[b] == usize::MAX) {
                    order[b] = next;
                    next += 1;
                    queue.push_back(b);
                }
            }
        }
    }
    order
}

/// How many tracks the drawn lines of a band take.
fn tracks_in(runs: &[Run], band: Band) -> i32 {
    runs.iter().filter(|r| r.band == Some(band)).filter_map(|r| r.track).map(|t| t as i32 + 1).max().unwrap_or(0)
}

/// Positions along one side for `n` lines, as the router spreads them.
fn spots(n: usize) -> Vec<u8> {
    match n {
        1 => vec![50],
        2 => vec![25, 75],
        3 => vec![25, 50, 75],
        _ => (1..=n).map(|k| (k * 100 / (n + 1)) as u8).collect(),
    }
}

/// The spot nearest `to`, the smaller on a tie.
fn nearest(spots: &[u8], to: u8) -> usize {
    (0..spots.len()).min_by_key(|&k| ((spots[k] as i32 - to as i32).abs(), spots[k])).expect("a spot is left")
}

fn bottom(r: Rect) -> i32 {
    r.y + r.h
}

fn probe(model: &Model, rules: &Rules, i: usize) -> Icon {
    probe_at(model, rules, i, Rect { x: 0, y: 0, w: ICON, h: ICON })
}

fn probe_at(model: &Model, rules: &Rules, i: usize, rect: Rect) -> Icon {
    let n = &model.nodes[i];
    let s = rules.service(&n.service).unwrap_or_else(|| panic!("model has unknown service `{}`", n.service));
    icon(n, s, rules, model.lang, rect, None)
}

trait WithParent {
    fn with_parent(self, parent: String) -> Self;
}
impl WithParent for Icon {
    fn with_parent(mut self, parent: String) -> Icon {
        self.parent = Some(parent);
        self
    }
}

#[cfg(test)]
pub(in crate::arrange) mod tests {
    use super::super::{arrange, Placement};
    use super::*;
    use crate::def::Rel;
    use crate::diag::Diagnostics;
    use crate::layout::Rect;
    use crate::measure::name_area;
    use crate::model::{Area, Edge, Node, Tier};
    use crate::path::hits;
    use crate::route::{port_point, Bend, Keep, Port, Side, LABEL_FIRST, LABEL_LINE};
    use crate::rules::Lang;
    use crate::test_inputs::{FEWER_READERS, WEB_CASES};
    use std::sync::OnceLock;

    pub(in crate::arrange) fn grid(src: &str) -> (Model, Diagnostics) {
        let (m, d) = crate::infer::infer(&crate::def::parse(src).unwrap(), &Rules::builtin());
        (m.unwrap_or_else(|| panic!("{:?}\n{src}", d.errors)), d)
    }

    fn node(id: &str, service: &str, part: &str, area: Area, az: Option<usize>, role: Option<&str>) -> Node {
        Node {
            id: id.into(),
            service: service.into(),
            option: None,
            az,
            role: role.map(String::from),
            added: false,
            region: None,
            part: part.into(),
            tag: part.split_once('.').map(|(_, t)| t.to_string()),
            area,
            undecided: false,
        }
    }
    fn edge(from: &str, to: &str, rel: Rel, label: &str, hole: Option<&str>) -> Edge {
        Edge { from: from.into(), to: to.into(), rel, label: label.into(), hole: hole.map(String::from), endpoint: false }
    }
    fn http_api() -> Node {
        Node { option: Some("http".into()), ..node("apigateway", "apigateway", "apigateway", Area::Region, None, None) }
    }

    /// No load balancer: Lambda copies and the Aurora Writer and Reader in the private tier, S3
    /// and CloudWatch outside the VPC.
    pub(in crate::arrange) fn smallest_mix(lambda: &str) -> Model {
        let pri = Area::Subnet(Tier::Private);
        let (l1, l2) = (crate::ids::copy_id(lambda, "1a"), crate::ids::copy_id(lambda, "1c"));
        let nodes = vec![
            node("user", "user", "user", Area::Outside, None, None),
            http_api(),
            node(&l1, "lambda", lambda, pri, Some(0), None),
            node(&l2, "lambda", lambda, pri, Some(1), None),
            node("aurora-1a", "aurora", "aurora", pri, Some(0), Some("writer")),
            node("aurora-1c", "aurora", "aurora", pri, Some(1), Some("reader")),
            node("s3", "s3", "s3", Area::Region, None, None),
            node("cloudwatch", "cloudwatch", "cloudwatch", Area::Region, None, None),
        ];
        let mut edges = vec![edge("user", "apigateway", Rel::Main, "https", None)];
        for l in [&l1, &l2] {
            edges.push(edge("apigateway", l, Rel::Main, "invoke", None));
        }
        for l in [&l1, &l2] {
            edges.push(edge(l, "aurora-1a", Rel::Main, "read_write", None));
        }
        for l in [&l1, &l2] {
            edges.push(edge(l, "aurora-1c", Rel::Main, "read", None));
        }
        edges.push(edge("aurora-1a", "aurora-1c", Rel::Aux, "replication", None));
        for l in [&l1, &l2] {
            edges.push(edge(l, "s3", Rel::Main, "read_write", Some("route.lambda.s3")));
        }
        for l in [&l1, &l2] {
            edges.push(edge(l, "cloudwatch", Rel::Aux, "logs_metrics", None));
        }
        Model { region: "ap-northeast-1".into(), azs: vec!["ap-northeast-1a".into(), "ap-northeast-1c".into()], lang: Lang::Ja, nodes, edges, direction: None }
    }

    /// The web example with API Gateway and Lambda copies in ECS's row, both reaching Aurora.
    pub(in crate::arrange) fn same_row() -> (Model, Diagnostics) {
        let (mut m, d) = grid(WEB_CASES[0].1);
        let pri = Area::Subnet(Tier::Private);
        m.nodes.push(http_api());
        m.nodes.push(node("lambda.api@1a", "lambda", "lambda.api", pri, Some(0), None));
        m.nodes.push(node("lambda.api@1c", "lambda", "lambda.api", pri, Some(1), None));
        m.edges.push(edge("user", "apigateway", Rel::Main, "https", None));
        for l in ["lambda.api@1a", "lambda.api@1c"] {
            m.edges.push(edge("apigateway", l, Rel::Main, "invoke", None));
        }
        for l in ["lambda.api@1a", "lambda.api@1c"] {
            m.edges.push(edge(l, "aurora-1a", Rel::Main, "read_write", None));
            m.edges.push(edge(l, "aurora-1c", Rel::Main, "read", None));
        }
        (m, d)
    }

    /// The same, with a Lambda function outside the VPC sending to the same S3 and CloudWatch.
    pub(in crate::arrange) fn shared_targets() -> (Model, Diagnostics) {
        let (mut m, d) = same_row();
        m.nodes.push(node("lambda.report", "lambda", "lambda.report", Area::Region, None, None));
        m.edges.push(edge("apigateway", "lambda.report", Rel::Main, "invoke", None));
        m.edges.push(edge("lambda.report", "s3", Rel::Main, "read_write", None));
        for l in ["lambda.api@1a", "lambda.api@1c", "lambda.report"] {
            m.edges.push(edge(l, "cloudwatch", Rel::Aux, "logs_metrics", None));
        }
        (m, d)
    }

    /// Three zones with a load balancer, and Lambda copies written before ECS in its row, so a
    /// Lambda copy takes the middle zone's first column, right below the load balancer.
    fn lambda_under_the_balancer() -> Model {
        let (mut m, _) = grid(WEB_CASES[2].1);
        let pri = Area::Subnet(Tier::Private);
        let at = m.nodes.iter().position(|n| n.part == "ecs").unwrap();
        for (k, zone) in ["1a", "1c", "1d"].iter().enumerate() {
            m.nodes.insert(at + k, node(&crate::ids::copy_id("lambda.api", zone), "lambda", "lambda.api", pri, Some(k), None));
        }
        m.nodes.push(http_api());
        m.edges.push(edge("user", "apigateway", Rel::Main, "https", None));
        for zone in ["1a", "1c", "1d"] {
            m.edges.push(edge("apigateway", &crate::ids::copy_id("lambda.api", zone), Rel::Main, "invoke", None));
        }
        m
    }

    /// The web example with WAF and ACM, which has no NAT gateway beside the load balancer, with
    /// a second load balancer beside the first, both reached from the entry gateway.
    fn two_balancers() -> Model {
        let (mut m, _) = grid(WEB_CASES[9].1);
        let alb = m.nodes.iter().find(|n| n.id == "alb").unwrap().clone();
        let at = m.nodes.iter().position(|n| n.id == "alb").unwrap();
        m.nodes.insert(at + 1, Node { id: "alb.b".into(), part: "alb.b".into(), tag: Some("b".into()), ..alb });
        m.edges.push(edge("igw", "alb.b", Rel::Main, "https", None));
        m
    }

    /// The web example with two zones: the load balancer sits on the zones' boundary.
    fn on_the_boundary(axis: Axis) -> Placement {
        arrange(&grid(WEB_CASES[0].1).0, &Rules::builtin(), axis).unwrap()
    }

    // Fails if the load balancer on the boundary takes a row of its own when the entry gateway's
    // line comes into it from above, or shares the NAT gateways' row when that line runs along it.
    #[test]
    fn the_boundary_row_holds_the_public_copies_when_entered_from_above() {
        let down = on_the_boundary(Axis::Down);
        let y = |p: &Placement, id: &str| icon(p, id).rect.y;
        assert_eq!((y(&down, "alb"), y(&down, "nat-1c")), (y(&down, "nat-1a"), y(&down, "nat-1a")));
        let right = on_the_boundary(Axis::Right);
        assert!(y(&right, "alb") < y(&right, "nat-1a"), "{} {}", y(&right, "alb"), y(&right, "nat-1a"));
        assert_eq!(y(&right, "igw"), y(&right, "alb"));
    }

    // Fails if the band above the private tier keeps a label's height when no line in it is one
    // the layout keeps room for a label for.
    #[test]
    fn a_band_keeping_no_label_room_is_as_tall_as_its_tracks() {
        let p = on_the_boundary(Axis::Down);
        let (pubf, pri) = (frame(&p, "pub-1a"), frame(&p, "pri-1a"));
        let fan = p.lanes.iter().find(|l| l.lo == pubf.y + pubf.h).expect("the band above the private tier");
        // Two fan lines, one track each: 10 (room 8 and the line's width 2) above, between and below.
        assert_eq!(fan.tracks.len(), 2, "{fan:?}");
        assert_eq!((fan.hi - fan.lo, pri.y - fan.lo), (30, 30));
        let mut at: Vec<i32> = fan.tracks.iter().map(|t| t.1).collect();
        at.sort();
        assert_eq!(at, [fan.lo + 10, fan.lo + 20]);
    }

    // Fails if the first thing in the VPC is not set just past the frame's name, or if the VPC
    // keeps the Region's margin for parts instead of the inset of a frame in a frame.
    #[test]
    fn the_vpc_sits_close_in_its_frames() {
        let p = on_the_boundary(Axis::Down);
        let (region, vpc, igw) = (frame(&p, "region"), frame(&p, "vpc"), icon(&p, "igw").rect);
        // The name box is 25 high; 8 of room below it.
        assert_eq!(igw.y - vpc.y, 33);
        // Across: the inset of a frame in a frame. Down: the rank also holds S3, so the Region's 60.
        assert_eq!((vpc.x - region.x, vpc.y - region.y), (30, 60));
    }

    // Fails if a gap down the page keeps its width across the page instead of the labels' height.
    #[test]
    fn gaps_down_the_page_hold_their_labels() {
        let p = on_the_boundary(Axis::Down);
        let (user, cloud, cf, region) = (icon(&p, "user"), frame(&p, "aws-cloud"), icon(&p, "cloudfront"), frame(&p, "region"));
        // The users start 40 from the top. The lines out of the users and out of CloudFront carry
        // one-line labels, 13 high, and 40 more of room.
        assert_eq!(user.rect.y, 40);
        let below = |i: &Icon| {
            let n = name_area(i);
            n.y + n.h
        };
        assert_eq!((cloud.y - below(user), region.y - below(cf)), (53, 53));
        // The global rank starts past the AWS Cloud frame's name (25) and 20 of room.
        assert_eq!(cf.rect.y - cloud.y, 45);
    }

    // Fails if a line the widening cannot clear is left through a part and the layout returned anyway.
    #[test]
    fn a_line_widening_cannot_clear_fails_the_layout() {
        let r = Rules::builtin();
        let m = lambda_under_the_balancer();
        let e = arrange(&m, &r, Axis::Down).unwrap_err();
        assert!(e.iter().any(|p| p.code == "LAYOUT_FAILED" && p.message.contains("edge-apigateway-to-lambda.api@1c") && p.message.contains("alb")), "{e:?}");
    }

    // Fails if two fixed ends are left on one spot of one side.
    #[test]
    fn two_fixed_ends_on_one_spot_fail_the_layout() {
        let r = Rules::builtin();
        let m = two_balancers();
        for axis in [Axis::Right, Axis::Down] {
            let e = arrange(&m, &r, axis).unwrap_err();
            assert!(e.iter().any(|p| p.code == "LAYOUT_FAILED" && p.message.contains("edge-igw-to-alb") && p.message.contains("edge-igw-to-alb.b")), "{axis:?} {e:?}");
        }
    }

    pub(in crate::arrange) const LONG: &str = "lambda.resize-images-for-the-we";

    /// Every model the inside of a VPC is laid out for, by name.
    pub(in crate::arrange) fn inputs() -> Vec<(String, Model, Diagnostics)> {
        let mut out: Vec<(String, Model, Diagnostics)> = WEB_CASES.iter().map(|(n, s)| {
            let (m, d) = grid(s);
            (n.to_string(), m, d)
        }).collect();
        let single = WEB_CASES[2].1.replace("  azs: 3\n", "  azs: 3\n  nat: single\n");
        let (m, d) = grid(&single);
        out.push(("web-3az-ja-single".into(), m, d));
        let (m, d) = grid(FEWER_READERS);
        out.push(("fewer-readers".into(), m, d));
        out.push(("mix-min".into(), smallest_mix("lambda"), Diagnostics::default()));
        let (m, d) = same_row();
        out.push(("mix-same-row".into(), m, d));
        let (m, d) = shared_targets();
        out.push(("mix-shared".into(), m, d));
        out.push(("mix-long".into(), smallest_mix(LONG), Diagnostics::default()));
        out
    }

    fn axis_name(a: Axis) -> &'static str {
        match a {
            Axis::Right => "right",
            Axis::Down => "down",
        }
    }
    /// One input laid out: (name, model, axis, the layout or its problems).
    type Laid = (String, Model, Axis, Result<Placement, Vec<Problem>>);

    /// Every input laid out along both axes, once for all the tests.
    fn laid_out() -> &'static [Laid] {
        static ALL: OnceLock<Vec<Laid>> = OnceLock::new();
        ALL.get_or_init(|| {
            let r = Rules::builtin();
            let mut out = Vec::new();
            for (name, m, _) in inputs() {
                for axis in [Axis::Right, Axis::Down] {
                    let label = format!("{name} {}", axis_name(axis));
                    let p = arrange(&m, &r, axis);
                    out.push((label, m.clone(), axis, p));
                }
            }
            out
        })
    }

    /// Every input laid out: (name, model, axis, placement).
    fn placements() -> impl Iterator<Item = (&'static String, &'static Model, Axis, &'static Placement)> {
        laid_out().iter().map(|(label, m, axis, p)| (label, m, *axis, p.as_ref().unwrap_or_else(|e| panic!("{label}: {e:?}"))))
    }

    fn frame(p: &Placement, id: &str) -> Rect {
        p.layout.frames.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("no frame {id}")).rect
    }
    fn icon<'a>(p: &'a Placement, id: &str) -> &'a Icon {
        p.layout.icons.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no icon {id}"))
    }
    fn suffix(m: &Model, az: usize) -> String {
        let n = &m.azs[az];
        n[n.len() - 2..].to_string()
    }
    fn inside(n: &Node) -> bool {
        matches!(n.area, Area::VpcEdge | Area::Subnet(_))
    }
    fn centre(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }
    /// Where a line meets an icon's side, below the name on the bottom side, as the router puts it.
    fn port_of(i: &Icon, side: Side, pos: u8) -> (i32, i32) {
        let offset = if side == Side::Bottom { LABEL_FIRST + LABEL_LINE * (i.lines as i32 - 1) } else { 0 };
        port_point(i.rect, Port { side, pos, offset })
    }
    /// The points of a kept draft: the whole line, or the gate and the inner part for a crossing line.
    fn kept_points(p: &Placement, pl: &LinePlan) -> Vec<(i32, i32)> {
        match &pl.keep {
            Keep::Whole => {
                let mut v = vec![port_of(icon(p, &pl.from), pl.exit, pl.exit_pos.unwrap())];
                if let Bend::Points(ps) = &pl.bend {
                    v.extend(ps.iter().copied());
                }
                v.push(port_of(icon(p, &pl.to), pl.entry, pl.entry_pos.unwrap()));
                v
            }
            Keep::Inner(t) if t.inner_is_start => {
                let mut v = vec![port_of(icon(p, &pl.from), pl.exit, pl.exit_pos.unwrap())];
                v.extend(t.inner.iter().copied());
                v.push(t.gate);
                v
            }
            Keep::Inner(t) => {
                let mut v = vec![t.gate];
                v.extend(t.inner.iter().copied());
                v.push(port_of(icon(p, &pl.to), pl.entry, pl.entry_pos.unwrap()));
                v
            }
            Keep::Free => Vec::new(),
        }
    }

    // Fails if parts overlap or leave their frames inside the VPC, or if a layout is only found
    // by widening up to the limit.
    #[test]
    fn vpc_layouts_pass_their_check() {
        let failed: Vec<String> = laid_out().iter().filter_map(|(label, _, _, p)| p.as_ref().err().map(|e| format!("{label}: {e:?}"))).collect();
        assert!(failed.is_empty(), "{}", failed.join("\n"));
        // A layout that ran out of widenings would have failed above; none comes near the limit.
        let most = WIDENED.load(std::sync::atomic::Ordering::Relaxed);
        assert!(most < TRIES / 10, "{most}");
    }

    // Fails if a part's copies sit at different heights, or at different places in their zones.
    #[test]
    fn copies_of_one_part_share_a_height() {
        for (label, m, _, p) in placements() {
            let mut parts: Vec<&str> = m.nodes.iter().filter(|n| n.az.is_some()).map(|n| n.part.as_str()).collect();
            parts.dedup();
            for part in parts {
                let copies: Vec<&Node> = m.nodes.iter().filter(|n| n.part == part && n.az.is_some()).collect();
                let at: Vec<(i32, i32)> = copies
                    .iter()
                    .map(|n| {
                        let r = icon(p, &n.id).rect;
                        (r.y, r.x - frame(p, &format!("az-{}", suffix(m, n.az.unwrap()))).x)
                    })
                    .collect();
                assert!(at.windows(2).all(|w| w[0] == w[1]), "{label}: {part} {at:?}");
            }
        }
    }

    // Fails if a zone's frame is sized by its own contents.
    #[test]
    fn zones_have_one_size() {
        for (label, _, _, p) in placements() {
            let zones: Vec<(i32, i32)> = p.layout.frames.iter().filter(|f| f.id.starts_with("az-")).map(|f| (f.rect.w, f.rect.h)).collect();
            assert!(zones.len() >= 2, "{label}");
            assert!(zones.windows(2).all(|w| w[0] == w[1]), "{label}: {zones:?}");
        }
    }

    // Fails if a subnet tier with nothing to place in it is drawn.
    #[test]
    fn a_tier_without_parts_has_no_frame() {
        for (label, m, _, p) in placements() {
            let count = |prefix: &str| p.layout.frames.iter().filter(|f| f.id.starts_with(prefix)).count();
            if label.starts_with("mix-min ") || label.starts_with("mix-long ") {
                assert_eq!((count("pub-"), count("pri-")), (0, 2), "{label}");
            } else if !label.starts_with("mix-") {
                assert_eq!((count("pub-"), count("pri-")), (m.azs.len(), m.azs.len()), "{label}");
            }
        }
    }

    // Fails if the entry gateway is not moved to the side the outside comes in from.
    #[test]
    fn the_internet_gateway_sits_on_the_entry_side() {
        for (label, m, axis, p) in placements() {
            for n in m.nodes.iter().filter(|n| n.area == Area::VpcEdge) {
                let g = icon(p, &n.id);
                let name = name_area(g);
                assert_eq!(g.parent.as_deref(), Some("vpc"), "{label}");
                for z in p.layout.frames.iter().filter(|f| f.id.starts_with("az-")) {
                    match axis {
                        Axis::Right => assert!(g.rect.x + g.rect.w < z.rect.x && name.x + name.w < z.rect.x, "{label}: {:?} {:?}", g.rect, z.rect),
                        Axis::Down => assert!(name.y + name.h < z.rect.y, "{label}: {:?} {:?}", g.rect, z.rect),
                    }
                }
            }
        }
    }

    // Fails if the part sending into the entry gateway is set to the middle of its targets instead.
    #[test]
    fn the_entry_runs_straight() {
        let mut seen = 0;
        for (label, m, axis, p) in placements() {
            if label.starts_with("mix-") || !m.nodes.iter().any(|n| n.id == "cloudfront") {
                continue;
            }
            seen += 1;
            let across = |id: &str| {
                let c = centre(icon(p, id).rect);
                if axis == Axis::Right { c.1 } else { c.0 }
            };
            assert_eq!(across("cloudfront"), across("igw"), "{label}");
            assert_eq!(across("alb"), across("igw"), "{label}");
        }
        assert!(seen > 0);
    }

    // Fails if the outside is laid out without reading the axis.
    #[test]
    fn the_outside_runs_along_the_axis() {
        for (label, _, axis, p) in placements() {
            let (u, cloud) = (icon(p, "user").rect, frame(p, "aws-cloud"));
            match axis {
                Axis::Right => assert!(u.x + u.w < cloud.x, "{label}"),
                Axis::Down => assert!(u.y + u.h < cloud.y, "{label}"),
            }
        }
    }

    // Fails if a line inside the VPC is left to the search or kept along the outside axis, or a
    // crossing line or an outside line is kept otherwise.
    #[test]
    fn drafts_keep_what_they_should() {
        for (label, m, axis, p) in placements() {
            for pl in &p.lines {
                let e = &m.edges[pl.edge];
                let at = |id: &str| inside(m.nodes.iter().find(|n| n.id == id).unwrap());
                match (at(&e.from), at(&e.to)) {
                    (true, true) => assert!(pl.keep == Keep::Whole && pl.axis == Axis::Down, "{label}: {pl:?}"),
                    (false, false) => assert!(pl.keep == Keep::Free && pl.axis == axis, "{label}: {pl:?}"),
                    _ => assert!(matches!(pl.keep, Keep::Inner(_)) && pl.axis == axis, "{label}: {pl:?}"),
                }
            }
        }
    }

    // Fails if a gate and the line through it disagree, if a gate is off the VPC frame's side, or
    // if a draft runs along the frame's side at its gate instead of crossing it at a right angle.
    #[test]
    fn crossings_match_their_drafts() {
        for (label, m, _, p) in placements() {
            let crossing = m.edges.iter().filter(|e| {
                let at = |id: &str| inside(m.nodes.iter().find(|n| n.id == id).unwrap());
                at(&e.from) != at(&e.to)
            });
            assert_eq!(crossing.count(), p.crossings.len(), "{label}");
            let v = frame(p, "vpc");
            for c in &p.crossings {
                assert_eq!(c.frame, "vpc", "{label}");
                let pl = p.lines.iter().find(|l| l.edge == c.edge).unwrap();
                assert_eq!(pl.keep, Keep::Inner(c.through.clone()), "{label}");
                let t = &c.through;
                let (side, pos) = if t.inner_is_start { (pl.exit, pl.exit_pos) } else { (pl.entry, pl.entry_pos) };
                assert_eq!((side, pos), (c.inner_side, Some(c.inner_pos)), "{label}");
                let (gx, gy) = t.gate;
                let on_x = (gx == v.x || gx == v.x + v.w) && v.y < gy && gy < v.y + v.h;
                let on_y = (gy == v.y || gy == v.y + v.h) && v.x < gx && gx < v.x + v.w;
                assert!(on_x != on_y, "{label}: gate {:?} off {v:?}", t.gate);
                // The stretch of the inner part next to the gate, in the line's direction.
                let pts = kept_points(p, pl);
                let (a, b) = if t.inner_is_start { (pts[pts.len() - 2], pts[pts.len() - 1]) } else { (pts[0], pts[1]) };
                let inward = |q: (i32, i32)| v.x < q.0 && q.0 < v.x + v.w && v.y < q.1 && q.1 < v.y + v.h;
                if on_x {
                    assert!(a.1 == b.1 && a.0 != b.0, "{label}: {pts:?}");
                } else {
                    assert!(a.0 == b.0 && a.1 != b.1, "{label}: {pts:?}");
                }
                let inner_end = if t.inner_is_start { a } else { b };
                assert!(inward(inner_end), "{label}: {pts:?}");
            }
        }
    }

    // Fails if two drawn lines take one height in a band, a track leaves its band, or a band keeps
    // a track for a line that is not drawn.
    #[test]
    fn tracks_in_a_band_keep_apart() {
        let r = Rules::builtin();
        for (label, m, _, p) in placements() {
            let reps: Vec<usize> = crate::lines::gather(m, &p.layout, &r).0.iter().map(|l| l.rep).collect();
            for lane in &p.lanes {
                assert!(lane.from < lane.to && lane.lo <= lane.hi, "{label}: {lane:?}");
                for (k, &(e, at)) in lane.tracks.iter().enumerate() {
                    assert!(lane.lo <= at && at <= lane.hi, "{label}: {lane:?}");
                    assert!(reps.contains(&e), "{label}: {e} in {lane:?}");
                    for &(_, other) in &lane.tracks[k + 1..] {
                        assert!((at - other).abs() >= 8, "{label}: {lane:?}");
                    }
                }
            }
        }
    }

    // Fails if a track is given to a line whose points are not rebuilt to run along it.
    #[test]
    fn tracked_drafts_lie_on_their_tracks() {
        let mut seen = 0;
        for (label, _, _, p) in placements() {
            for pl in p.lines.iter().filter(|l| !l.tracks.is_empty()) {
                let pts = kept_points(p, pl);
                for t in &pl.tracks {
                    seen += 1;
                    assert!(t.horizontal && t.from < t.to, "{label}: {t:?}");
                    let on = pts.windows(2).any(|s| s[0].1 == t.at && s[1].1 == t.at && s[0].0.min(s[1].0) == t.from && s[0].0.max(s[1].0) == t.to);
                    assert!(on, "{label}: {t:?} {pts:?}");
                    let lane = p.lanes.iter().find(|l| l.tracks.contains(&(pl.edge, t.at)));
                    assert!(lane.is_some(), "{label}: {t:?} in no band");
                }
            }
        }
        assert!(seen > 0);
    }

    // Fails if a line running straight is given a track of no length, or a band keeps it.
    #[test]
    fn a_straight_line_takes_no_track() {
        let (m, _) = grid(WEB_CASES[2].1);
        let r = Rules::builtin();
        for axis in [Axis::Right, Axis::Down] {
            let p = arrange(&m, &r, axis).unwrap();
            let straight = m.edges.iter().position(|e| e.from == "alb" && e.to == "ecs-1c").unwrap();
            let pl = p.lines.iter().find(|l| l.edge == straight).unwrap();
            assert_eq!(pl.bend, Bend::Straight, "{pl:?}");
            assert!(pl.tracks.is_empty(), "{pl:?}");
            assert!(p.lanes.iter().all(|l| l.tracks.iter().all(|t| t.0 != straight)), "{:?}", p.lanes);
            let bending = m.edges.iter().position(|e| e.from == "alb" && e.to == "ecs-1a").unwrap();
            assert_eq!(p.lines.iter().find(|l| l.edge == bending).unwrap().tracks.len(), 1);
        }
    }

    // Fails if a merged line's representative has no draft, or an edge has two.
    #[test]
    fn drafts_cover_every_edge() {
        for (label, m, _, p) in placements() {
            let edges: Vec<usize> = p.lines.iter().map(|l| l.edge).collect();
            assert_eq!(edges, (0..m.edges.len()).collect::<Vec<_>>(), "{label}");
        }
    }

    // Fails if the VPC does not come first in its rank, or if where it goes follows the order
    // lines are written in.
    #[test]
    fn the_vpc_comes_first_in_its_rank() {
        let r = Rules::builtin();
        let src = WEB_CASES[0].1;
        let moved = src.replace("  - cloudfront -> s3\n", "").replace("flows:\n", "flows:\n  - cloudfront -> s3\n");
        assert_ne!(src, moved);
        for axis in [Axis::Right, Axis::Down] {
            for s in [src, moved.as_str()] {
                let p = arrange(&grid(s).0, &r, axis).unwrap();
                let (s3, v) = (icon(&p, "s3").rect, frame(&p, "vpc"));
                // S3 shares the VPC's rank: past the VPC across the main axis, below it when lines
                // run right and right of it when they run down.
                match axis {
                    Axis::Right => assert!(s3.y > v.y + v.h, "{s3:?} {v:?}"),
                    Axis::Down => assert!(s3.x > v.x + v.w, "{s3:?} {v:?}"),
                }
            }
        }
    }

    // Fails if the layout takes a load balancer for granted.
    #[test]
    fn the_vpc_does_not_lean_on_a_load_balancer() {
        let r = Rules::builtin();
        let m = smallest_mix("lambda");
        for axis in [Axis::Right, Axis::Down] {
            let p = arrange(&m, &r, axis).unwrap();
            assert_eq!(icon(&p, "lambda-1a").parent.as_deref(), Some("pri-1a"));
            assert_eq!(icon(&p, "lambda-1c").parent.as_deref(), Some("pri-1c"));
        }
    }

    /// The y of every horizontal stretch of the drafts of the given edges.
    fn level_ys(p: &Placement, m: &Model, pick: impl Fn(&Edge) -> bool) -> Vec<i32> {
        let mut out = Vec::new();
        for pl in p.lines.iter().filter(|l| pick(&m.edges[l.edge])) {
            let pts = kept_points(p, pl);
            out.extend(pts.windows(2).filter(|s| s[0].1 == s[1].1 && s[0].0 != s[1].0).map(|s| s[0].1));
        }
        out
    }

    // Fails if two sets of lines from one row to the zones' Aurora copies share their tracks.
    #[test]
    fn two_sets_of_mesh_lines_take_their_own_lanes() {
        let r = Rules::builtin();
        let (m, _) = same_row();
        for axis in [Axis::Right, Axis::Down] {
            let p = arrange(&m, &r, axis).unwrap();
            assert_eq!(icon(&p, "ecs-1a").rect.y, icon(&p, "lambda.api@1a").rect.y);
            let across = |part: &str| {
                level_ys(&p, &m, |e| {
                    let f = m.nodes.iter().find(|n| n.id == e.from).unwrap();
                    let t = m.nodes.iter().find(|n| n.id == e.to).unwrap();
                    f.part == part && t.part == "aurora" && f.az != t.az
                })
            };
            let (ecs, lambda) = (across("ecs"), across("lambda.api"));
            assert!(!ecs.is_empty() && !lambda.is_empty());
            assert!(ecs.iter().all(|y| !lambda.contains(y)), "{ecs:?} {lambda:?}");
        }
    }

    // Fails if senders in different places or of different services are merged, a line is dropped,
    // or the shared targets make the layout overlap.
    #[test]
    fn shared_targets_inside_and_outside_stay_apart() {
        use crate::lines::Source;
        let r = Rules::builtin();
        let (m, _) = shared_targets();
        for axis in [Axis::Right, Axis::Down] {
            let p = arrange(&m, &r, axis).unwrap();
            let lines = crate::lines::gather(&m, &p.layout, &r).0;
            let into: Vec<Vec<Source>> = lines.iter().filter(|l| l.to == "cloudwatch").map(|l| l.sources.clone()).collect();
            let want = vec![
                vec![Source { part: "ecs".into(), azs: vec![0, 1] }],
                vec![Source { part: "lambda.api".into(), azs: vec![0, 1] }],
                vec![Source { part: "lambda.report".into(), azs: vec![] }],
            ];
            assert_eq!(into, want);
        }
    }

    // Fails if the room around the lines is not worked out from the names' sizes.
    #[test]
    fn long_names_widen_the_lanes() {
        let r = Rules::builtin();
        let m = smallest_mix(LONG);
        for axis in [Axis::Right, Axis::Down] {
            let p = arrange(&m, &r, axis).unwrap();
            for zone in ["1a", "1c"] {
                let name = name_area(icon(&p, &crate::ids::copy_id(LONG, zone)));
                for pl in &p.lines {
                    let pts = kept_points(&p, pl);
                    for s in pts.windows(2) {
                        assert!(!hits(s[0], s[1], name), "{zone} {pl:?} {pts:?} {name:?}");
                    }
                }
            }
        }
    }

    /// Lays `m` out along `axis` and draws its lines from the drafts: the layout must hold, no
    /// line may come out broken, and every semantic edge must be in exactly one drawn line.
    fn lays_out_cleanly(name: &str, m: &Model, d: &Diagnostics, axis: Axis) -> Placement {
        let r = Rules::builtin();
        let p = arrange(m, &r, axis).unwrap_or_else(|e| panic!("{name} {axis:?}: {e:?}"));
        let (_, notes) = crate::arranged_drawio(m, d, &r, axis).unwrap();
        let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
        assert!(broken.is_empty(), "{name} {axis:?}: {broken:?}");
        let mut edges: Vec<usize> = crate::lines::gather(m, &p.layout, &r).0.iter().flat_map(|l| l.edges.clone()).collect();
        edges.sort_unstable();
        assert_eq!(edges, (0..m.edges.len()).collect::<Vec<_>>(), "{name} {axis:?}");
        p
    }

    // Fails if three zones with Aurora copies in only two of them overlap, break, leave a draft or drop a line.
    #[test]
    fn three_zones_with_one_reader_lay_out_cleanly() {
        let (m, d) = grid(FEWER_READERS);
        for axis in [Axis::Right, Axis::Down] {
            lays_out_cleanly("fewer readers", &m, &d, axis);
        }
    }

    // Fails if three zones with one NAT gateway overlap, break, leave a draft or drop a line.
    #[test]
    fn three_zones_with_one_nat_lay_out_cleanly() {
        let (m, d) = grid(&WEB_CASES[2].1.replace("  azs: 3\n", "  azs: 3\n  nat: single\n"));
        assert_eq!(m.nodes.iter().filter(|n| n.part == "nat").count(), 1);
        for axis in [Axis::Right, Axis::Down] {
            lays_out_cleanly("one NAT gateway", &m, &d, axis);
        }
    }

    // Fails if the web example with WAF and ACM overlaps, breaks, leaves a draft or drops a line.
    #[test]
    fn web_with_waf_and_acm_lays_out_cleanly() {
        let (m, d) = grid(WEB_CASES[9].1);
        for axis in [Axis::Right, Axis::Down] {
            lays_out_cleanly("web-attach-en", &m, &d, axis);
        }
    }

    // Fails if four Lambda functions with long names and two CloudWatch parts, running down,
    // overlap, break, leave a draft or drop a line.
    #[test]
    fn many_long_named_functions_lay_out_cleanly_down() {
        let src = "flows:\n  - user -> apigateway(http) -> lambda.resize-images-fast -> dynamodb.images\n  - apigateway -> lambda.thumbnails-generator -> dynamodb.images\n  - apigateway -> lambda.metadata-extractor -> dynamodb.meta\n  - apigateway -> lambda.notification-sender -> dynamodb.meta\n  - lambda.resize-images-fast ~> cloudwatch.app\n  - lambda.thumbnails-generator ~> cloudwatch.app\n  - lambda.metadata-extractor ~> cloudwatch.app\n  - lambda.metadata-extractor ~> cloudwatch.audit\n  - lambda.notification-sender ~> cloudwatch.audit\nset:\n  direction: down\n";
        let r = Rules::builtin();
        let (m, d) = crate::flow_infer::infer(&crate::def::parse(src).unwrap(), &r);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        assert_eq!(m.nodes.iter().filter(|n| n.service == "lambda").count(), 4);
        lays_out_cleanly("long names", &m, &d, Axis::Down);
    }

    // Fails if, with two zones and one NAT gateway, the line from the other zone's copy into it
    // overlaps, breaks or leaves its draft, or shares a track with the lines fanning out below.
    #[test]
    fn two_zones_with_one_nat_lay_out_cleanly() {
        let (m, d) = grid(&WEB_CASES[0].1.replace("set:\n", "set:\n  nat: single\n"));
        assert_eq!(m.nodes.iter().filter(|n| n.part == "nat").count(), 1);
        let across = m.edges.iter().position(|e| e.from == "ecs-1c" && e.to == "nat-1a").expect("a line to the other zone's NAT gateway");
        for axis in [Axis::Right, Axis::Down] {
            let p = lays_out_cleanly("two zones, one NAT gateway", &m, &d, axis);
            let pl = p.lines.iter().find(|l| l.edge == across).unwrap();
            // Out of the side facing the NAT gateway's zone, up the gap between the zones.
            let (left, right) = (frame(&p, "az-1a"), frame(&p, "az-1c"));
            let Bend::Points(pts) = &pl.bend else { panic!("{pl:?}") };
            assert_eq!(pl.exit, Side::Left, "{axis:?}");
            assert!(pts[0].0 > left.x + left.w && pts[0].0 < right.x, "{axis:?} {pts:?}");
            let t = pl.tracks.first().expect("a track");
            let fan = p.lanes.iter().find(|l| l.tracks.contains(&(across, t.at))).expect("its band");
            assert_eq!(fan.tracks.iter().filter(|x| x.1 == t.at).count(), 1, "{axis:?} {fan:?}");
        }
    }

    // Fails if the line to the other zone's NAT gateway is drawn along its row through a part
    // beside it there, instead of up from its top.
    #[test]
    fn a_part_beside_sends_the_line_to_the_other_zones_nat_gateway_up() {
        let (mut m, d) = grid(&WEB_CASES[0].1.replace("set:\n", "set:\n  nat: single\n"));
        let pri = Area::Subnet(Tier::Private);
        let at = m.nodes.iter().position(|n| n.part == "ecs").unwrap();
        for (k, zone) in ["1a", "1c"].iter().enumerate() {
            m.nodes.insert(at + k, node(&crate::ids::copy_id("lambda.api", zone), "lambda", "lambda.api", pri, Some(k), None));
            m.edges.push(edge("alb", &crate::ids::copy_id("lambda.api", zone), Rel::Main, "http", None));
        }
        let across = m.edges.iter().position(|e| e.from == "ecs-1c" && e.to == "nat-1a").unwrap();
        for axis in [Axis::Right, Axis::Down] {
            let p = lays_out_cleanly("a part beside", &m, &d, axis);
            assert!(icon(&p, "lambda.api@1c").rect.x < icon(&p, "ecs-1c").rect.x);
            assert_eq!(p.lines.iter().find(|l| l.edge == across).unwrap().exit, Side::Top, "{axis:?}");
        }
    }

    // Fails if, with two zones, a NAT gateway per zone and a side line out of the load balancer
    // in their row, that line overlaps or breaks, or the load balancer leaves the NAT gateways' row.
    #[test]
    fn a_side_line_out_of_the_balancer_beside_the_nat_gateways_lays_out_cleanly() {
        let (m, d) = grid(&WEB_CASES[0].1.replace("  - ecs ~> cloudwatch\n", "  - alb ~> cloudwatch\n"));
        let side = m.edges.iter().position(|e| e.from == "alb" && e.to == "cloudwatch").unwrap();
        let p = lays_out_cleanly("side line out of the load balancer", &m, &d, Axis::Down);
        assert_eq!(icon(&p, "alb").rect.y, icon(&p, "nat-1c").rect.y);
        let c = p.crossings.iter().find(|c| c.edge == side).expect("a crossing");
        // Down out of the load balancer, along a track below the public subnets, out by the right side.
        assert_eq!(c.inner_side, Side::Bottom);
        assert!(c.through.inner.iter().all(|q| q.1 > frame(&p, "pub-1c").y + frame(&p, "pub-1c").h), "{c:?}");
        lays_out_cleanly("side line out of the load balancer", &m, &d, Axis::Right);
    }

    // Fails if a line from outside the Region into a part below or beside the VPC is drafted
    // through the VPC, or breaks.
    #[test]
    fn a_line_into_the_region_goes_around_the_vpc() {
        let src = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - cloudfront -> s3\n  - ecs -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\nset:\n  nat: single\n";
        let (m, d) = grid(src);
        let line = m.edges.iter().position(|e| e.from == "cloudfront" && e.to == "s3").unwrap();
        for axis in [Axis::Right, Axis::Down] {
            let p = lays_out_cleanly("line around the VPC", &m, &d, axis);
            let pl = p.lines.iter().find(|l| l.edge == line).unwrap();
            let Bend::Points(pts) = &pl.bend else { panic!("{axis:?} {pl:?}") };
            let vpc = frame(&p, "vpc");
            assert!(pts.windows(2).all(|s| !hits(s[0], s[1], vpc)), "{axis:?} {pts:?}");
        }
    }

    // Fails if writing a side line before the main lines moves a part on the main lines.
    #[test]
    fn side_line_order_moves_no_main_part() {
        let src = WEB_CASES[0].1;
        let moved = src.replace("  - ecs ~> cloudwatch\n", "").replace("flows:\n", "flows:\n  - ecs ~> cloudwatch\n");
        assert_ne!(src, moved);
        let ((a, da), (b, db)) = (grid(src), grid(&moved));
        for axis in [Axis::Right, Axis::Down] {
            let (pa, pb) = (lays_out_cleanly("web-2az-ja", &a, &da, axis), lays_out_cleanly("side line first", &b, &db, axis));
            let main: Vec<&str> = a.edges.iter().filter(|e| e.rel == Rel::Main).flat_map(|e| [e.from.as_str(), e.to.as_str()]).collect();
            assert!(main.contains(&"alb") && main.contains(&"ecs-1a"));
            for id in main {
                assert_eq!(icon(&pa, id).rect, icon(&pb, id).rect, "{axis:?} {id}");
            }
        }
    }

    // Fails if a line the router draws from the new layout's drafts comes out broken: through a
    // part, a name or another line, or off its draft or its track.
    #[test]
    fn lines_drawn_from_the_vpc_drafts_keep_clear() {
        let r = Rules::builtin();
        let mut broken = Vec::new();
        for (label, m, _, p) in placements() {
            let (_, notes) = crate::route::route_planned(m, &p.layout, &r, &p.lines);
            let named: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
            if !named.is_empty() {
                broken.push(format!("{label}: {named:?}"));
            }
        }
        assert!(broken.is_empty(), "{}", broken.join("\n"));
    }

    /// With `ARCHWIRE_SAMPLE_OUT=<dir>`, writes every input laid out along both axes as
    /// `<dir>/<input>-<axis>.drawio`, for the diagram checker. Without it, does nothing.
    #[test]
    fn vpc_samples() {
        let Some(dir) = std::env::var_os("ARCHWIRE_SAMPLE_OUT") else { return };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = Rules::builtin();
        let mut report = String::new();
        for (name, m, d) in inputs() {
            for axis in [Axis::Right, Axis::Down] {
                let file = format!("{name}-{}", axis_name(axis));
                let (x, notes) = crate::arranged_drawio(&m, &d, &r, axis).unwrap_or_else(|e| panic!("{file}: {e:?}"));
                let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
                report.push_str(&format!("{file}\t{}\t{broken:?}\n", broken.len()));
                std::fs::write(dir.join(format!("{file}.drawio")), x).unwrap();
            }
        }
        std::fs::write(dir.join("notes.tsv"), report).unwrap();
    }
}
