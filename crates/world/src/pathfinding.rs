//! Shortest paths between squares (`Square._shortest_path_to`,
//! `World._create_graphs` and `lib/priodict.py`).
//!
//! Ground units travel through exits: the ground graph links the exits of a
//! square to each other and each exit to its other side. Air and water
//! units travel from square to square.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use crate::entity::{EntityId, Place, PlayerId, SquareId};
use crate::nofloat::int_distance;
use crate::rules::AirGround;
use crate::world::{Abort, INF, Res, World};

/// A vertex of a path graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
    Square(SquareId),
    Exit(EntityId),
}

type Graph = BTreeMap<Node, Vec<(Node, i64)>>;

/// The path graphs of the three layers.
#[derive(Clone, Debug, Default)]
pub struct Graphs {
    ground: Graph,
    air: Graph,
    water: Graph,
}

/// The result of a shortest path search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PathResult {
    /// The next exit or square and the distance, or `None` and [`INF`].
    Next(Option<Node>, i64),
    /// The squares of the exits on the way.
    Places(Vec<SquareId>),
}

type CacheKey = (SquareId, SquareId, AirGround, Option<PlayerId>, bool, bool);

/// The results computed during the current world time. The original keeps
/// them until the time changes, even if exits get blocked meanwhile.
#[derive(Clone, Debug, Default)]
pub struct PathCache {
    time: Option<i64>,
    results: BTreeMap<CacheKey, PathResult>,
}

/// Who asks whether an exit is blocked: nobody, a player, or a unit. A
/// gate lets its friends through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Viewer {
    Nobody,
    Player(PlayerId),
    Unit(EntityId),
}

impl World {
    pub(crate) fn create_graphs(&mut self) {
        let mut ground = Graph::new();
        for s in 0..self.squares.len() {
            let exits = self.squares[s].exits.clone();
            for &e in &exits {
                let (ex, ey) = (self.e(e).x, self.e(e).y);
                let mut edges = Vec::new();
                for &f in &exits {
                    if f != e {
                        edges.push((
                            Node::Exit(f),
                            int_distance(ex, ey, self.e(f).x, self.e(f).y),
                        ));
                    }
                }
                if let Some(os) = self.exit_other_side(e) {
                    edges.push((Node::Exit(os), 0));
                }
                ground.insert(Node::Exit(e), edges);
            }
        }
        let air = self.layer_graph(|sq| sq.is_air);
        let water = self.layer_graph(|sq| sq.is_water);
        self.graphs = Graphs { ground, air, water };
    }

    fn layer_graph(&self, open: impl Fn(&crate::square::Square) -> bool) -> Graph {
        let mut g = Graph::new();
        for (i, z) in self.squares.iter().enumerate() {
            let mut edges = Vec::new();
            if open(z) {
                // Some diagonals are missing next to a closed square.
                let n = if z.strict_neighbors.iter().any(|&s| !open(self.square(s))) {
                    &z.strict_neighbors
                } else {
                    &z.neighbors
                };
                for &z2 in n {
                    let s2 = self.square(z2);
                    if open(s2) {
                        edges.push((Node::Square(z2), int_distance(z.x, z.y, s2.x, s2.y)));
                    }
                }
            }
            g.insert(Node::Square(SquareId(i as u32)), edges);
        }
        g
    }

    /// The other side of an exit.
    pub fn exit_other_side(&self, e: EntityId) -> Option<EntityId> {
        self.e(e).exit.as_ref().and_then(|x| x.other_side)
    }

    /// The blockers of both sides of an exit.
    pub(crate) fn exit_blockers(&self, e: EntityId) -> Vec<EntityId> {
        let mut b = self
            .e(e)
            .exit
            .as_ref()
            .map(|x| x.blockers.clone())
            .unwrap_or_default();
        if let Some(os) = self.exit_other_side(e) {
            b.extend(
                self.e(os)
                    .exit
                    .as_ref()
                    .map(|x| x.blockers.clone())
                    .unwrap_or_default(),
            );
        }
        b
    }

    /// `Exit.is_blocked`: a dense forest, or a blocker that is not a gate or
    /// is not a friend of the viewer. With `ignore_enemy_walls`, enemy
    /// blockers do not count.
    pub(crate) fn exit_is_blocked(
        &self,
        e: EntityId,
        viewer: Viewer,
        ignore_enemy_walls: bool,
        ignore_forests: bool,
    ) -> bool {
        let forest = self
            .e(e)
            .exit
            .as_ref()
            .is_some_and(|x| x.is_blocked_by_forests);
        if !ignore_forests && forest {
            return true;
        }
        for b in self.exit_blockers(e) {
            let enemy = match viewer {
                Viewer::Nobody => true,
                Viewer::Player(p) => self.player_is_an_enemy_of_entity(p, self.e(b)),
                Viewer::Unit(u) => self.is_an_enemy(u, &crate::entity::Target::Entity(b)),
            };
            if ignore_enemy_walls && enemy {
                continue;
            }
            if self.e(b).stats.is_a_gate == 0 || enemy {
                return true;
            }
        }
        false
    }

    fn graph(&self, plane: AirGround) -> &Graph {
        match plane {
            AirGround::Ground => &self.graphs.ground,
            AirGround::Air => &self.graphs.air,
            AirGround::Water => &self.graphs.water,
        }
    }

    fn node_xy_id(&self, n: Node) -> (i64, i64, i64) {
        match n {
            Node::Square(s) => {
                let sq = self.square(s);
                (sq.x, sq.y, i64::from(sq.id))
            }
            Node::Exit(e) => (self.e(e).x, self.e(e).y, i64::from(e.0)),
        }
    }

    /// `shortest_path_to`: the next exit (ground) or square (air, water) on
    /// the shortest path, or `None` if there is no path or `start` is
    /// `dest`.
    pub fn shortest_path_to(
        &mut self,
        start: SquareId,
        dest: SquareId,
        player: Option<PlayerId>,
        plane: AirGround,
        avoid: bool,
    ) -> Res<Option<Node>> {
        match self.shortest_path(start, dest, plane, player, false, avoid)? {
            PathResult::Next(n, _) => Ok(n),
            PathResult::Places(_) => Ok(None),
        }
    }

    /// `shortest_path_distance_to`: the distance, or [`INF`] without a path.
    pub fn shortest_path_distance_to(
        &mut self,
        start: SquareId,
        dest: SquareId,
        player: Option<PlayerId>,
        plane: AirGround,
        avoid: bool,
    ) -> Res<i64> {
        match self.shortest_path(start, dest, plane, player, false, avoid)? {
            PathResult::Next(_, d) => Ok(d),
            PathResult::Places(_) => Ok(INF),
        }
    }

    /// `shortest_path_to(..., places=True)`: the squares of the exits on
    /// the way (ground), or of the squares of the way (air, water).
    pub fn shortest_path_places(
        &mut self,
        start: SquareId,
        dest: SquareId,
        player: Option<PlayerId>,
        plane: AirGround,
        avoid: bool,
    ) -> Result<Vec<SquareId>, Abort> {
        match self.shortest_path(start, dest, plane, player, true, avoid)? {
            PathResult::Places(p) => Ok(p),
            PathResult::Next(..) => Ok(Vec::new()),
        }
    }

    fn shortest_path(
        &mut self,
        start: SquareId,
        dest: SquareId,
        plane: AirGround,
        player: Option<PlayerId>,
        places: bool,
        avoid: bool,
    ) -> Res<PathResult> {
        if self.path_cache.time != Some(self.time) {
            self.path_cache.results.clear();
            self.path_cache.time = Some(self.time);
        }
        let key = (start, dest, plane, player, places, avoid);
        if let Some(r) = self.path_cache.results.get(&key) {
            return Ok(r.clone());
        }
        let r = self.dijkstra(start, dest, plane, player, places, avoid)?;
        self.path_cache.results.insert(key, r.clone());
        Ok(r)
    }

    fn is_avoided(&self, player: Option<PlayerId>, avoid: bool, n: Node) -> Res<bool> {
        if !avoid {
            return Ok(false);
        }
        let Some(p) = player else {
            return Err(Abort::new(
                "'NoneType' object has no attribute 'is_very_dangerous'",
            ));
        };
        Ok(match n {
            Node::Square(s) => self.square_is_very_dangerous(p, s),
            Node::Exit(e) => self.exit_is_very_dangerous(p, e),
        })
    }

    fn is_blocked_node(&self, player: Option<PlayerId>, n: Node) -> bool {
        match n {
            Node::Exit(e) => {
                let v = player.map_or(Viewer::Nobody, Viewer::Player);
                self.exit_is_blocked(e, v, true, false)
            }
            Node::Square(_) => false,
        }
    }

    /// The neighbors of a vertex, with the start and the destination
    /// squares linked to their exits on the ground layer.
    fn neighbors_of(&self, n: Node, plane: AirGround, ends: [SquareId; 2]) -> Vec<(Node, i64)> {
        let mut out = Vec::new();
        if plane == AirGround::Ground {
            if let Node::Square(s) = n {
                if ends.contains(&s) {
                    let sq = self.square(s);
                    for &e in &sq.exits {
                        out.push((
                            Node::Exit(e),
                            int_distance(sq.x, sq.y, self.e(e).x, self.e(e).y),
                        ));
                    }
                }
                return out;
            }
            if let Some(edges) = self.graph(plane).get(&n) {
                out.extend_from_slice(edges);
            }
            if let Node::Exit(e) = n
                && let Some(Place::Square(s)) = self.e(e).place
            {
                for end in ends {
                    if end == s && self.square(end).exits.contains(&e) {
                        let sq = self.square(end);
                        out.push((
                            Node::Square(end),
                            int_distance(sq.x, sq.y, self.e(e).x, self.e(e).y),
                        ));
                    }
                }
            }
            return out;
        }
        if let Some(edges) = self.graph(plane).get(&n) {
            out.extend_from_slice(edges);
        }
        out
    }

    fn dijkstra(
        &self,
        start: SquareId,
        dest: SquareId,
        plane: AirGround,
        player: Option<PlayerId>,
        places: bool,
        avoid: bool,
    ) -> Res<PathResult> {
        if dest == start {
            return Ok(if places {
                PathResult::Places(vec![start])
            } else {
                PathResult::Next(None, 0)
            });
        }
        let ends = [start, dest];
        let start_n = Node::Square(start);
        let end_n = Node::Square(dest);
        // Priorities are (distance, id); the start has (0,), smaller than
        // any (0, id). The id makes the result the same on any machine.
        type Prio = (i64, Option<i64>);
        let mut final_d: BTreeMap<Node, i64> = BTreeMap::new();
        let mut pred: BTreeMap<Node, Node> = BTreeMap::new();
        let mut queue: BTreeMap<Node, Prio> = BTreeMap::new();
        let mut heap: BinaryHeap<Reverse<(Prio, Node)>> = BinaryHeap::new();
        queue.insert(start_n, (0, None));
        heap.push(Reverse(((0, None), start_n)));
        while let Some(Reverse((prio, v))) = heap.pop() {
            if queue.get(&v) != Some(&prio) {
                continue;
            }
            let skip = self.is_blocked_node(player, v) || self.is_avoided(player, avoid, v)?;
            if !skip {
                final_d.insert(v, prio.0);
                if v == end_n {
                    break;
                }
                for (w, len) in self.neighbors_of(v, plane, ends) {
                    if self.is_blocked_node(player, w) || self.is_avoided(player, avoid, w)? {
                        continue;
                    }
                    let vw = prio.0 + len;
                    if final_d.contains_key(&w) {
                        continue;
                    }
                    let better = match queue.get(&w) {
                        None => true,
                        Some(q) => vw < q.0,
                    };
                    if better {
                        let p = (vw, Some(self.node_xy_id(w).2));
                        queue.insert(w, p);
                        heap.push(Reverse((p, w)));
                        pred.insert(w, v);
                    }
                }
            }
            queue.remove(&v);
        }
        if !pred.contains_key(&end_n) {
            return Ok(if places {
                PathResult::Places(Vec::new())
            } else {
                PathResult::Next(None, INF)
            });
        }
        let mut path = vec![end_n];
        let mut cur = end_n;
        while cur != start_n {
            cur = pred[&cur];
            path.push(cur);
        }
        path.reverse();
        if places {
            let squares = path
                .iter()
                .filter_map(|n| match n {
                    Node::Exit(e) => self.e(*e).square(),
                    Node::Square(_) => None,
                })
                .collect();
            return Ok(PathResult::Places(squares));
        }
        match final_d.get(&end_n) {
            Some(&d) => Ok(PathResult::Next(Some(path[1]), d)),
            None => Err(Abort::new(format!("KeyError: {}", self.square(dest).name))),
        }
    }
}
