//! The model behind an asset browser: what is listed, and in what order.
//!
//! A baked pack is a flat list of ids. That is fine for code and useless for a
//! person: by the time a pack holds a hundred assets, finding "the dark
//! cratered one" means reading every id. This module is the part that turns
//! the list into something browsable — text search, facet filters, sorting —
//! and it is deliberately free of Bevy so the rules can be tested directly
//! rather than through a UI.
//!
//! The widget that DRAWS it belongs to the game (engine ADR-0002 puts screens
//! in the game repository): games build the items, lay out the panel, and feed
//! clicks back into a [`BrowserQuery`]. Pair it with [`crate::preview`] for the
//! turntable and [`crate::thumbnail`] for the tiles.

use std::collections::{BTreeMap, BTreeSet};

/// One row in the browser.
///
/// `group` and `tags` are whatever taxonomy the GAME authored — the engine has
/// no opinion about what a family is. `AssetRecord::category` is a reasonable
/// default for `category`, but a game with a richer scheme is free to ignore it.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserItem {
    /// Asset id, as it appears in the pack.
    pub id: String,
    /// Human-facing label. Falls back to the id when a game has nothing better.
    pub name: String,
    /// Broad bucket, e.g. an `AssetCategory` or a game's own split.
    pub category: String,
    /// Finer bucket within a category, e.g. "chondrite", "precursor".
    pub group: String,
    pub tags: Vec<String>,
    pub triangles: u32,
    /// Longest-axis size in metres, for sorting by how big a thing is.
    pub size_m: f32,
    /// Free text that should match search but is never displayed as a label —
    /// a generator prompt, an author note, a source filename.
    pub searchable_notes: String,
}

impl BrowserItem {
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            name: id.clone(),
            id,
            category: String::new(),
            group: String::new(),
            tags: Vec::new(),
            triangles: 0,
            size_m: 0.0,
            searchable_notes: String::new(),
        }
    }

    /// Every field a text query is allowed to hit.
    fn matches_text(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        // Callers lowercase the needle once; doing it per item per keystroke
        // showed up as jank on a 100-item grid.
        let hit = |s: &str| s.to_lowercase().contains(needle);
        hit(&self.name)
            || hit(&self.id)
            || hit(&self.group)
            || hit(&self.category)
            || hit(&self.searchable_notes)
            || self.tags.iter().any(|t| hit(t))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Name,
    /// Group first, then name inside it — how you read a catalogue.
    Group,
    Triangles,
    Size,
}

impl SortKey {
    pub const ALL: [SortKey; 4] = [
        SortKey::Name,
        SortKey::Group,
        SortKey::Triangles,
        SortKey::Size,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Group => "Family",
            SortKey::Triangles => "Tris",
            SortKey::Size => "Size",
        }
    }
}

/// What the browser is currently showing.
///
/// Filters are AND across kinds (category AND group AND every selected tag)
/// and the tag set is AND within itself: picking "mineable" and "ice" means
/// assets that are both, which is the only reading that lets a filter list
/// narrow rather than flail.
#[derive(Debug, Clone, Default)]
pub struct BrowserQuery {
    pub text: String,
    pub category: Option<String>,
    pub group: Option<String>,
    pub tags: BTreeSet<String>,
    pub sort: SortKey,
    pub descending: bool,
}

impl BrowserQuery {
    pub fn is_filtered(&self) -> bool {
        !self.text.is_empty()
            || self.category.is_some()
            || self.group.is_some()
            || !self.tags.is_empty()
    }

    pub fn clear_filters(&mut self) {
        self.text.clear();
        self.category = None;
        self.group = None;
        self.tags.clear();
    }

    /// Toggle a tag on or off. Selecting an active tag clears it, so the same
    /// click that narrowed the list widens it again.
    pub fn toggle_tag(&mut self, tag: &str) {
        if !self.tags.remove(tag) {
            self.tags.insert(tag.to_string());
        }
    }

    /// Toggle an exclusive facet: picking the active value clears it.
    fn toggle_one(slot: &mut Option<String>, value: &str) {
        if slot.as_deref() == Some(value) {
            *slot = None;
        } else {
            *slot = Some(value.to_string());
        }
    }

    pub fn toggle_category(&mut self, value: &str) {
        Self::toggle_one(&mut self.category, value);
        // A group belongs to a category; keeping a stale one selected produces
        // an empty grid that looks like a bug rather than a filter.
        self.group = None;
    }

    pub fn toggle_group(&mut self, value: &str) {
        Self::toggle_one(&mut self.group, value);
    }

    fn accepts(&self, item: &BrowserItem, needle: &str) -> bool {
        if let Some(c) = &self.category {
            if &item.category != c {
                return false;
            }
        }
        if let Some(g) = &self.group {
            if &item.group != g {
                return false;
            }
        }
        if !self
            .tags
            .iter()
            .all(|want| item.tags.iter().any(|t| t == want))
        {
            return false;
        }
        item.matches_text(needle)
    }

    /// Indices of the matching items, in display order.
    ///
    /// Indices rather than references so a caller can hold the result across a
    /// frame without borrowing the item list.
    pub fn apply(&self, items: &[BrowserItem]) -> Vec<usize> {
        let needle = self.text.trim().to_lowercase();
        let mut out: Vec<usize> = (0..items.len())
            .filter(|&i| self.accepts(&items[i], &needle))
            .collect();

        out.sort_by(|&a, &b| {
            let (x, y) = (&items[a], &items[b]);
            let ord = match self.sort {
                SortKey::Name => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
                SortKey::Group => x
                    .group
                    .to_lowercase()
                    .cmp(&y.group.to_lowercase())
                    .then_with(|| x.name.to_lowercase().cmp(&y.name.to_lowercase())),
                SortKey::Triangles => x.triangles.cmp(&y.triangles),
                // Sizes are measured floats and can be NaN if a manifest is
                // malformed; total_cmp orders them anyway instead of panicking
                // or silently producing an inconsistent sort.
                SortKey::Size => x.size_m.total_cmp(&y.size_m),
            };
            // Ties break on id so the grid never reshuffles between frames.
            let ord = ord.then_with(|| x.id.cmp(&y.id));
            if self.descending {
                ord.reverse()
            } else {
                ord
            }
        });
        out
    }
}

/// The filter sidebar's contents: every value that exists, and how many items
/// carry it.
///
/// Counts come from the items the OTHER filters already accepted, so the
/// sidebar answers "what happens if I click this" rather than advertising
/// facets that would return nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facets {
    pub categories: Vec<(String, usize)>,
    pub groups: Vec<(String, usize)>,
    pub tags: Vec<(String, usize)>,
}

fn tally(counts: BTreeMap<String, usize>) -> Vec<(String, usize)> {
    counts.into_iter().collect()
}

/// Build the facet lists for `items` under `query`.
pub fn facets(items: &[BrowserItem], query: &BrowserQuery) -> Facets {
    let needle = query.text.trim().to_lowercase();

    // Each facet is counted with ITS OWN filter suppressed. Counting a facet
    // against the full query makes every unselected option in that facet read
    // zero the moment one is picked, which is useless for switching between
    // them.
    let mut without_category = query.clone();
    without_category.category = None;
    without_category.group = None;
    let mut without_group = query.clone();
    without_group.group = None;

    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();

    for item in items {
        if without_category.accepts(item, &needle) && !item.category.is_empty() {
            *categories.entry(item.category.clone()).or_default() += 1;
        }
        if without_group.accepts(item, &needle) && !item.group.is_empty() {
            *groups.entry(item.group.clone()).or_default() += 1;
        }
        // Tags stay counted under the full query: they combine with AND, so
        // "how many of what I'm looking at also have this tag" is the useful
        // number.
        if query.accepts(item, &needle) {
            for tag in &item.tags {
                *tags.entry(tag.clone()).or_default() += 1;
            }
        }
    }

    Facets {
        categories: tally(categories),
        groups: tally(groups),
        tags: tally(tags),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, cat: &str, group: &str, tags: &[&str], tris: u32, size: f32) -> BrowserItem {
        BrowserItem {
            tags: tags.iter().map(|s| s.to_string()).collect(),
            category: cat.to_string(),
            group: group.to_string(),
            triangles: tris,
            size_m: size,
            ..BrowserItem::new(id)
        }
    }

    fn corpus() -> Vec<BrowserItem> {
        vec![
            item("rock.ice", "rock", "ice", &["mineable", "field"], 100, 70.0),
            item("rock.iron", "rock", "metallic", &["mineable"], 300, 60.0),
            item("rock.dust", "rock", "regolith", &["field"], 200, 90.0),
            item("exo.gate", "exotic", "gate", &["landmark"], 400, 150.0),
        ]
    }

    fn ids(items: &[BrowserItem], picked: Vec<usize>) -> Vec<&str> {
        picked.iter().map(|&i| items[i].id.as_str()).collect()
    }

    #[test]
    fn empty_query_returns_everything_sorted_by_name() {
        let items = corpus();
        let picked = BrowserQuery::default().apply(&items);
        assert_eq!(
            ids(&items, picked),
            ["exo.gate", "rock.dust", "rock.ice", "rock.iron"]
        );
    }

    #[test]
    fn text_search_covers_tags_and_notes_not_just_names() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.text = "mineable".into();
        assert_eq!(ids(&items, q.apply(&items)), ["rock.ice", "rock.iron"]);

        let mut notes = corpus();
        notes[3].searchable_notes = "an ancient mineable relic".into();
        assert!(q.apply(&notes).len() == 3, "notes should be searchable");
    }

    #[test]
    fn search_is_case_insensitive() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.text = "MiNeAbLe".into();
        assert_eq!(q.apply(&items).len(), 2);
    }

    #[test]
    fn tags_combine_with_and_not_or() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.toggle_tag("mineable");
        q.toggle_tag("field");
        // Only rock.ice carries both.
        assert_eq!(ids(&items, q.apply(&items)), ["rock.ice"]);
    }

    #[test]
    fn toggling_a_tag_twice_clears_it() {
        let mut q = BrowserQuery::default();
        q.toggle_tag("field");
        assert!(q.is_filtered());
        q.toggle_tag("field");
        assert!(!q.is_filtered());
    }

    #[test]
    fn changing_category_drops_a_group_that_no_longer_applies() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.toggle_group("ice");
        q.toggle_category("exotic");
        assert_eq!(q.group, None, "stale group would have emptied the grid");
        assert_eq!(ids(&items, q.apply(&items)), ["exo.gate"]);
    }

    #[test]
    fn sorting_by_size_and_triangles_respects_direction() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.sort = SortKey::Triangles;
        assert_eq!(
            ids(&items, q.apply(&items)),
            ["rock.ice", "rock.dust", "rock.iron", "exo.gate"]
        );
        q.descending = true;
        assert_eq!(
            ids(&items, q.apply(&items)),
            ["exo.gate", "rock.iron", "rock.dust", "rock.ice"]
        );
        q.sort = SortKey::Size;
        q.descending = false;
        assert_eq!(
            ids(&items, q.apply(&items)),
            ["rock.iron", "rock.ice", "rock.dust", "exo.gate"]
        );
    }

    #[test]
    fn sort_is_stable_against_nan_sizes() {
        let mut items = corpus();
        items[0].size_m = f32::NAN;
        let mut q = BrowserQuery::default();
        q.sort = SortKey::Size;
        // The point is that it produces a full, deterministic ordering rather
        // than panicking or dropping rows.
        assert_eq!(q.apply(&items).len(), 4);
        assert_eq!(q.apply(&items), q.apply(&items));
    }

    #[test]
    fn facet_counts_ignore_their_own_filter_so_you_can_switch_between_them() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.toggle_category("rock");
        let f = facets(&items, &q);
        // "exotic" must still show its count, or picking it is impossible.
        assert_eq!(
            f.categories,
            vec![("exotic".to_string(), 1), ("rock".to_string(), 3)]
        );
    }

    #[test]
    fn facet_groups_narrow_to_the_selected_category() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.toggle_category("exotic");
        let f = facets(&items, &q);
        assert_eq!(f.groups, vec![("gate".to_string(), 1)]);
    }

    #[test]
    fn tag_facet_counts_reflect_the_current_result_set() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.toggle_category("rock");
        let f = facets(&items, &q);
        assert_eq!(
            f.tags,
            vec![("field".to_string(), 2), ("mineable".to_string(), 2)]
        );
    }

    #[test]
    fn clear_filters_restores_the_full_list() {
        let items = corpus();
        let mut q = BrowserQuery::default();
        q.text = "ice".into();
        q.toggle_category("rock");
        q.toggle_tag("mineable");
        assert_eq!(q.apply(&items).len(), 1);
        q.clear_filters();
        assert_eq!(q.apply(&items).len(), items.len());
    }
}
