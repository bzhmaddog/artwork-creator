//! Changes to the layer tree: naming, duplicating, deleting, grouping, clipping,
//! moving, aligning, turning and flipping. No interface code.

use super::*;

/// Gives a node a new id.
pub(super) fn set_id(node: &mut Node, id: u64) {
    match node {
        Node::Group(g) => g.id = id,
        Node::Layer(l) => l.id = id,
    }
}

/// The node's opacity, layer or group.
pub(super) fn opacity_mut(node: &mut Node) -> &mut f32 {
    match node {
        Node::Group(g) => &mut g.opacity,
        Node::Layer(l) => &mut l.opacity,
    }
}

/// The node's name, layer or group.
pub(super) fn name_mut(node: &mut Node) -> &mut String {
    match node {
        Node::Group(g) => &mut g.name,
        Node::Layer(l) => &mut l.name,
    }
}

/// Where a new layer goes: at the top of a selected group, just above a selected layer,
/// or at the top of the stack.
pub fn insertion_path(doc: &mut Document, selected: Option<u64>) -> Vec<usize> {
    let Some(mut path) = selected.and_then(|id| doc.path_of(id)) else {
        return vec![doc.layers.len()];
    };
    if let Some(Node::Group(g)) = doc.node_at_mut(&path) {
        g.expanded = true;
        path.push(g.children.len());
    } else if let Some(last) = path.last_mut() {
        *last += 1;
    }
    path
}

/// Renames `node`, about to be added to `doc` as a copy, when `doc` has a layer or group
/// of that name already: "name copy", then "name copy (2)", "(3)"... A copy of a copy is
/// named after the original, not "name copy copy".
pub fn name_copy(doc: &Document, node: &mut Node) {
    let names = all_names(doc);
    let name = name_mut(node);
    if !names.contains(&name.as_str()) {
        return;
    }
    let base = copy_base(name).to_owned();
    *name = (1..)
        .map(|n| match n {
            1 => tr!("DUPLICATE_LAYER_NAME", name = base),
            n => tr!("DUPLICATE_LAYER_NAME_NUMBERED", name = base, n = n),
        })
        .find(|candidate| !names.contains(&candidate.as_str()))
        .expect("an unused name");
}

/// The names of the layers and groups of `doc`.
pub(super) fn all_names(doc: &Document) -> Vec<&str> {
    fn collect<'a>(nodes: &'a [Node], names: &mut Vec<&'a str>) {
        for n in nodes {
            match n {
                Node::Group(g) => {
                    names.push(&g.name);
                    collect(&g.children, names);
                }
                Node::Layer(l) => names.push(&l.name),
            }
        }
    }
    let mut names = Vec::new();
    collect(&doc.layers, &mut names);
    names
}

/// Two words picked at random from the language's lists, an adjective and a noun put
/// together by `LAYER_NAME_PATTERN` ("Swift Otter"), to name a new shape or canvas, or to
/// be a new text's placeholder: a name no layer or group of `doc` has yet, unless many
/// tries find none.
pub fn random_name(doc: &Document) -> String {
    use std::hash::BuildHasher;
    let list = |text: &'static str| -> Vec<&'static str> {
        text.split(',')
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .collect()
    };
    let adjectives = list(tr!("LAYER_NAME_ADJECTIVES"));
    let nouns = list(tr!("LAYER_NAME_NOUNS"));
    if adjectives.is_empty() || nouns.is_empty() {
        return String::new();
    }
    let names = all_names(doc);
    // Randomly keyed for each call: enough to pick names.
    let state = std::collections::hash_map::RandomState::new();
    let now = std::time::SystemTime::now();
    let pick = |k: u32, n: usize| (state.hash_one((k, now)) % n as u64) as usize;
    let mut name = String::new();
    for k in 0..64 {
        let adjective = adjectives[pick(2 * k, adjectives.len())];
        let noun = nouns[pick(2 * k + 1, nouns.len())];
        name = tr!("LAYER_NAME_PATTERN", adjective = adjective, noun = noun);
        if !names.contains(&name.as_str()) {
            break;
        }
    }
    name
}

/// The original's name, for a name made by `name_copy`.
pub(super) fn copy_base(name: &str) -> &str {
    // The texts around the name in each pattern, found by filling it with a marker.
    const MARK: &str = "\u{1}";
    let plain = tr!("DUPLICATE_LAYER_NAME", name = MARK);
    let numbered = tr!("DUPLICATE_LAYER_NAME_NUMBERED", name = MARK, n = MARK);
    let strip = |pattern: &str| -> Option<&str> {
        let (before, after) = pattern.split_once(MARK)?;
        let rest = name.strip_prefix(before)?;
        match after.split_once(MARK) {
            // Numbered: base, then text, digits, text.
            Some((mid, end)) => {
                let rest = rest.strip_suffix(end)?;
                let (base, digits) = rest.rsplit_once(mid)?;
                (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(base)
            }
            None => rest.strip_suffix(after),
        }
    };
    strip(&numbered)
        .or_else(|| strip(&plain))
        .filter(|base| !base.is_empty())
        .unwrap_or(name)
}

/// Gives `node` and everything in it new ids from `doc`, for a copy about to be added to
/// it. A text following a shape copied along with it follows the copy; one following a
/// shape `doc` doesn't have (pasted from another project) stops following it.
pub fn give_fresh_ids(doc: &mut Document, node: &mut Node) {
    let mut new_ids = std::collections::HashMap::new();
    node.for_each_node_mut(&mut |n| {
        let id = doc.alloc_id();
        new_ids.insert(n.id(), id);
        set_id(n, id);
    });
    node.for_each_layer_mut(&mut |l| {
        if let Some(t) = l.text.as_mut().filter(|t| t.path.shape != 0) {
            t.path.shape = match new_ids.get(&t.path.shape) {
                Some(&id) => id,
                None if doc.layer(t.path.shape).is_some_and(|s| s.shape.is_some()) => t.path.shape,
                None => 0,
            };
        }
    });
}

/// Copies the selected node just above itself, moved 5 mm, with new ids, and selects the
/// copy.
pub fn duplicate(doc: &mut Document, selected: &mut Option<u64>) -> bool {
    let Some(mut path) = selected.and_then(|id| doc.path_of(id)) else {
        return false;
    };
    let Some(mut copy) = doc.node_at(&path).cloned() else {
        return false;
    };
    give_fresh_ids(doc, &mut copy);
    copy.for_each_layer_mut(&mut |l| {
        l.transform.center[0] += 5.0;
        l.transform.center[1] += 5.0;
    });
    name_copy(doc, &mut copy);
    *selected = Some(copy.id());
    *path.last_mut().unwrap() += 1;
    doc.insert(&path, copy);
    true
}

/// Deletes the selected node and selects the one that took its place, the one below, or its
/// group.
pub fn delete(doc: &mut Document, selected: &mut Option<u64>) -> bool {
    let Some((path, _)) = selected.and_then(|id| doc.remove(id)) else {
        return false;
    };
    let (&i, parent) = path.split_last().unwrap();
    let siblings = doc.list(parent).map(Vec::as_slice).unwrap_or_default();
    // Next selection: the node that took its place, the one below, or the parent group.
    *selected = siblings
        .get(i)
        .or(i.checked_sub(1).and_then(|j| siblings.get(j)))
        .map(Node::id)
        .or_else(|| doc.node_at(parent).map(Node::id));
    true
}

/// Selects the row below the selected one in the layer list (above it when `down` is
/// false), as a click on it would; the first or last row when none is selected.
/// Returns true if the selection changed.
pub fn step_selection(doc: &Document, selected: &mut Option<u64>, down: bool) -> bool {
    let rows = rows(doc);
    let at = selected.and_then(|id| rows.iter().position(|r| r.id == id));
    let next = match at {
        Some(i) if down => rows.get(i + 1),
        Some(i) => i.checked_sub(1).and_then(|i| rows.get(i)),
        None if down => rows.first(),
        None => rows.last(),
    };
    match next {
        Some(row) => {
            *selected = Some(row.id);
            true
        }
        None => false,
    }
}

/// Moves the selected layer, or every unlocked layer of the selected group.
pub fn nudge(doc: &mut Document, selected: Option<u64>, d: P2) -> bool {
    let Some(id) = selected else { return false };
    let ids = movable_layers(doc, id);
    for &id in &ids {
        if let Some(l) = doc.layer_mut(id) {
            l.transform.center[0] += d[0];
            l.transform.center[1] += d[1];
        }
    }
    !ids.is_empty()
}

/// Whether the X and the Y of the layers moving with the selected node are linked to a
/// variable: the arrow keys leave a linked axis as it is.
pub fn linked_axes(doc: &Document, selected: Option<u64>) -> [bool; 2] {
    let ids = selected.map_or_else(Vec::new, |id| movable_layers(doc, id));
    [
        crate::document::variables::Field::X,
        crate::document::variables::Field::Y,
    ]
    .map(|field| {
        ids.iter().any(|&id| {
            doc.layer(id)
                .is_some_and(|l| l.bindings.contains_key(&field))
        })
    })
}

/// Page-mm bounds (min, max) of the given layers as drawn, rotation and distortion
/// included.
pub fn layers_bounds(doc: &Document, ids: &[u64]) -> Option<(P2, P2)> {
    ids.iter()
        .filter_map(|&id| doc.layer(id))
        .flat_map(|l| l.corners())
        .fold(None, |acc, p| {
            Some(match acc {
                None => (p, p),
                Some((lo, hi)) => (
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                ),
            })
        })
}

/// Moves the selected layer, or the unlocked layers of the selected group as one block,
/// so that its bounding box lines up with `area` (min, max corners in page mm).
pub fn align(doc: &mut Document, selected: Option<u64>, how: Align, area: (P2, P2)) -> bool {
    let Some(id) = selected else { return false };
    let ids = movable_layers(doc, id);
    let Some((lo, hi)) = layers_bounds(doc, &ids) else {
        return false;
    };
    let (alo, ahi) = area;
    let mid = |a: f32, b: f32| (a + b) / 2.0;
    let d = match how {
        Align::Left => [alo[0] - lo[0], 0.0],
        Align::CenterX => [mid(alo[0], ahi[0]) - mid(lo[0], hi[0]), 0.0],
        Align::Right => [ahi[0] - hi[0], 0.0],
        Align::Top => [0.0, alo[1] - lo[1]],
        Align::CenterY => [0.0, mid(alo[1], ahi[1]) - mid(lo[1], hi[1])],
        Align::Bottom => [0.0, ahi[1] - hi[1]],
    };
    if d == [0.0, 0.0] {
        return false;
    }
    nudge(doc, selected, d)
}

/// `t` turned by `deg` (clockwise) around `pivot` in page mm: its center goes around,
/// its rotation adds up (its flips, tilt and distortion, under the rotation, turn with
/// it).
pub fn turned_about(
    mut t: crate::document::Transform,
    pivot: P2,
    deg: f32,
) -> crate::document::Transform {
    let (sin, cos) = deg.to_radians().sin_cos();
    let d = [t.center[0] - pivot[0], t.center[1] - pivot[1]];
    t.center = [
        pivot[0] + d[0] * cos - d[1] * sin,
        pivot[1] + d[0] * sin + d[1] * cos,
    ];
    t.rotation_deg = (t.rotation_deg + deg + 180.0).rem_euclid(360.0) - 180.0;
    t
}

/// The middle of what the unlocked layers of node `id` cover: where a group turns and
/// is mirrored around.
pub fn group_middle(doc: &Document, id: u64) -> Option<P2> {
    let (lo, hi) = layers_bounds(doc, &movable_layers(doc, id))?;
    Some([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0])
}

/// Turns the unlocked layers of group `id` by `deg` (clockwise) around its middle.
/// Returns whether anything turned.
pub fn rotate_group(doc: &mut Document, id: u64, deg: f32) -> bool {
    let Some(pivot) = group_middle(doc, id) else {
        return false;
    };
    if deg % 360.0 == 0.0 {
        return false;
    }
    for id in movable_layers(doc, id) {
        if let Some(l) = doc.layer_mut(id) {
            l.transform = turned_about(l.transform, pivot, deg);
        }
    }
    true
}

/// Mirrors the unlocked layers of group `id` across the middle of what they cover, left
/// to right (`horizontally`) or top to bottom: each one's center goes to the other side,
/// its turn the other way, and its own flip is switched (its tilt and distortion, under
/// the flip, mirror with it). Returns whether anything moved.
pub fn flip_group(doc: &mut Document, id: u64, horizontally: bool) -> bool {
    let ids = movable_layers(doc, id);
    let Some((lo, hi)) = layers_bounds(doc, &ids) else {
        return false;
    };
    let (axis, mid) = if horizontally {
        (0, (lo[0] + hi[0]) / 2.0)
    } else {
        (1, (lo[1] + hi[1]) / 2.0)
    };
    for id in ids {
        let Some(l) = doc.layer_mut(id) else { continue };
        let t = &mut l.transform;
        t.center[axis] = 2.0 * mid - t.center[axis];
        t.rotation_deg = -t.rotation_deg;
        if horizontally {
            t.flip_x ^= true;
        } else {
            t.flip_y ^= true;
        }
    }
    true
}

/// Ids of the image layers that move with node `id`: itself or the unlocked layers it
/// holds, but backgrounds, which stay on the outline.
pub fn movable_layers(doc: &Document, id: u64) -> Vec<u64> {
    let Some(path) = doc.path_of(id) else {
        return Vec::new();
    };
    if doc.effectively_locked(id) {
        return Vec::new();
    }
    doc.leaves()
        .into_iter()
        .filter(|leaf| leaf.path.starts_with(&path) && !leaf.locked)
        .filter(|leaf| !crate::document::background::is_fixed(leaf.layer))
        .map(|leaf| leaf.layer.id)
        .collect()
}

/// Wraps the selected node in a new group, or adds an empty group at the top.
pub fn group(doc: &mut Document, selected: &mut Option<u64>) -> bool {
    let id = doc.alloc_id();
    fn count(nodes: &[Node]) -> usize {
        nodes
            .iter()
            .filter_map(Node::as_group)
            .map(|g| 1 + count(&g.children))
            .sum()
    }
    let count = count(&doc.layers);
    let mut group = Group {
        id,
        name: tr!("DEFAULT_GROUP_NAME", n = count + 1),
        visible: true,
        locked: false,
        opacity: 1.0,
        clip: false,
        blend: crate::document::BlendMode::Normal,
        expanded: true,
        children: Vec::new(),
        element: None,
    };
    let path = match selected.and_then(|s| doc.remove(s)) {
        Some((path, mut node)) => {
            // The group takes the node's place over the layer below, clipped or not.
            group.clip = std::mem::take(node.clip_mut());
            group.children.push(node);
            path
        }
        None => vec![doc.layers.len()],
    };
    doc.insert(&path, Node::Group(group));
    *selected = Some(id);
    true
}

/// Replaces the selected group by its children. Hidden, locked and opacity carry over
/// to the children so they look about the same.
pub fn ungroup(doc: &mut Document, selected: &mut Option<u64>) -> bool {
    if selected.and_then(|id| doc.node(id)?.as_group()).is_none() {
        return false;
    }
    let Some((mut path, Node::Group(g))) = selected.and_then(|id| doc.remove(id)) else {
        return false;
    };
    *selected = g.children.last().map(Node::id);
    for mut child in g.children {
        if !g.visible {
            match &mut child {
                Node::Group(c) => c.visible = false,
                Node::Layer(c) => c.visible = false,
            }
        }
        if g.locked {
            match &mut child {
                Node::Group(c) => c.locked = true,
                Node::Layer(c) => c.locked = true,
            }
        }
        *opacity_mut(&mut child) *= g.opacity;
        // Clipped as the group was, to the layer below it.
        *child.clip_mut() |= g.clip;
        doc.insert(&path, child);
        *path.last_mut().unwrap() += 1;
    }
    true
}

/// Whether the node can be clipped to the layer below: there is one in its list.
pub fn can_clip(doc: &Document, id: u64) -> bool {
    doc.path_of(id)
        .and_then(|p| p.last().copied())
        .is_some_and(|i| i > 0)
}

/// Turns "Clip to layer below" on or off for the node. Off always works; on needs a
/// layer below (`can_clip`).
pub fn toggle_clip(doc: &mut Document, id: Option<u64>) -> bool {
    let Some(id) = id else {
        return false;
    };
    let can = can_clip(doc, id);
    let Some(clip) = doc.node_mut(id).map(Node::clip_mut) else {
        return false;
    };
    if !*clip && !can {
        return false;
    }
    *clip ^= true;
    true
}

/// Whether `move_step` can move the node up (`up`) or down.
pub fn can_step(doc: &Document, id: u64, up: bool) -> bool {
    let Some(path) = doc.path_of(id) else {
        return false;
    };
    let (&i, parent) = path.split_last().unwrap();
    let len = doc.list(parent).map_or(0, Vec::len);
    let at_end = if up { i + 1 == len } else { i == 0 };
    !at_end || !parent.is_empty()
}

/// Moves a node one step up or down the stack. It enters a neighboring group, and
/// leaves its own group when it reaches the group's end.
pub fn move_step(doc: &mut Document, id: u64, up: bool) -> bool {
    if !can_step(doc, id, up) {
        return false;
    }
    let path = doc.path_of(id).unwrap();
    let (&i, parent) = path.split_last().unwrap();
    let list = doc.list(parent).unwrap();
    let neighbor = if up { i + 1 } else { i.wrapping_sub(1) };
    let Some(next) = list.get(neighbor) else {
        // Leave the group: just above or below it in its parent.
        let (&gi, grand) = parent.split_last().unwrap();
        let (_, node) = doc.remove(id).unwrap();
        let mut to = grand.to_vec();
        to.push(if up { gi + 1 } else { gi });
        doc.insert(&to, node);
        return true;
    };
    match next {
        Node::Group(_) => {
            let gid = next.id();
            let (_, node) = doc.remove(id).unwrap();
            let gpath = doc.path_of(gid).unwrap();
            let Some(Node::Group(g)) = doc.node_at_mut(&gpath) else {
                unreachable!()
            };
            g.expanded = true;
            let at = if up { 0 } else { g.children.len() };
            g.children.insert(at, node);
        }
        Node::Layer(_) => {
            doc.list_mut(parent).unwrap().swap(i, neighbor);
        }
    }
    true
}

/// Moves a node to the top (`top`) or bottom of its own list.
pub fn move_to_end(doc: &mut Document, id: u64, top: bool) -> bool {
    let Some(path) = doc.path_of(id) else {
        return false;
    };
    let (&i, parent) = path.split_last().unwrap();
    let list = doc.list_mut(parent).unwrap();
    let to = if top { list.len() - 1 } else { 0 };
    if i == to {
        return false;
    }
    let node = list.remove(i);
    list.insert(to, node);
    true
}

/// Applies a drag-and-drop move. Refuses to drop a group into itself.
pub fn drop_node(doc: &mut Document, id: u64, target: DropTarget) -> bool {
    let target_id = match target {
        DropTarget::Above(t) | DropTarget::Below(t) | DropTarget::Into(t) => t,
    };
    if doc.is_within(target_id, id) || doc.path_of(target_id).is_none() {
        return false;
    }
    let Some((from, node)) = doc.remove(id) else {
        return false;
    };
    let mut to = doc.path_of(target_id).unwrap();
    match target {
        DropTarget::Above(_) => *to.last_mut().unwrap() += 1,
        DropTarget::Below(_) => {}
        DropTarget::Into(_) => {
            let Some(Node::Group(g)) = doc.node_at_mut(&to) else {
                doc.insert(&from, node);
                return false;
            };
            g.expanded = true;
            to.push(g.children.len());
        }
    }
    doc.insert(&to, node);
    true
}

/// Whether a drag-and-drop move is allowed. In a template (`template_mode`), elements
/// stay at the top of the list and everything else inside an element: a shape goes from
/// an element to another, never out of them.
pub fn drop_allowed(doc: &Document, id: u64, target: DropTarget, template_mode: bool) -> bool {
    if !template_mode {
        return true;
    }
    // The reference stays at the bottom, under everything.
    let reference = |t: u64| crate::template::edit::is_reference(doc, t);
    if reference(id) || matches!(target, DropTarget::Below(t) | DropTarget::Into(t) if reference(t))
    {
        return false;
    }
    let parent = match target {
        DropTarget::Above(t) | DropTarget::Below(t) => doc.path_of(t).map(|mut p| {
            p.pop();
            p
        }),
        DropTarget::Into(t) => doc.path_of(t),
    };
    parent.is_some_and(|p| crate::template::edit::fits_in(doc, id, &p))
}
