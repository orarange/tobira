use std::marker::PhantomData;

use super::value::{JsObject, JsString};

const DEFAULT_PAGE_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeapArena {
    String,
    Object,
    Custom(&'static str),
}

pub trait ArenaItem {
    const ARENA: HeapArena;

    /// Bytes this item owns outside its own slot. Counting slots alone is not
    /// enough to decide when to collect: two thousand strings of a thousand
    /// characters are two megabytes but only two thousand slots, so a
    /// count-based trigger sails straight past them.
    fn footprint(&self) -> usize {
        0
    }
}

impl ArenaItem for JsString {
    const ARENA: HeapArena = HeapArena::String;

    fn footprint(&self) -> usize {
        self.text.len()
    }
}

impl ArenaItem for JsObject {
    const ARENA: HeapArena = HeapArena::Object;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawGcRef {
    arena: HeapArena,
    page_index: u32,
    slot_index: u32,
    generation: u32,
}

impl RawGcRef {
    pub const fn new(arena: HeapArena, page_index: u32, slot_index: u32, generation: u32) -> Self {
        Self {
            arena,
            page_index,
            slot_index,
            generation,
        }
    }

    pub const fn arena(self) -> HeapArena {
        self.arena
    }

    pub const fn page_index(self) -> u32 {
        self.page_index
    }

    pub const fn slot_index(self) -> u32 {
        self.slot_index
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct GcRef<T: ArenaItem> {
    raw: RawGcRef,
    _marker: PhantomData<fn() -> T>,
}

impl<T: ArenaItem> GcRef<T> {
    pub const fn new(page_index: u32, slot_index: u32, generation: u32) -> Self {
        Self {
            raw: RawGcRef::new(T::ARENA, page_index, slot_index, generation),
            _marker: PhantomData,
        }
    }

    pub const fn raw(self) -> RawGcRef {
        self.raw
    }

    pub const fn arena(self) -> HeapArena {
        self.raw.arena()
    }

    pub const fn page_index(self) -> u32 {
        self.raw.page_index()
    }

    pub const fn slot_index(self) -> u32 {
        self.raw.slot_index()
    }

    pub const fn generation(self) -> u32 {
        self.raw.generation()
    }
}

impl<T: ArenaItem> Clone for GcRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ArenaItem> Copy for GcRef<T> {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GcColor {
    #[default]
    White,
    Gray,
    Black,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeapHeader {
    mark_color: GcColor,
    /// Set instead of freeing when `TOBIRA_GC_VERIFY` is on: the collector
    /// decided this cell was unreachable, but left it in place. Any later read
    /// of it is a root the collector failed to see, and `Arena::get` says so
    /// out loud. See `Heap::verify_mode`.
    condemned: bool,
}

impl Default for HeapHeader {
    fn default() -> Self {
        Self {
            condemned: false,
            mark_color: GcColor::White,
        }
    }
}

impl HeapHeader {
    pub fn mark_color(&self) -> GcColor {
        self.mark_color
    }

    pub fn set_mark_color(&mut self, mark_color: GcColor) {
        self.mark_color = mark_color;
    }

    pub fn is_condemned(&self) -> bool {
        self.condemned
    }
}

#[derive(Debug, Clone)]
pub struct HeapCell<T> {
    header: HeapHeader,
    value: T,
}

impl<T> HeapCell<T> {
    pub fn new(value: T) -> Self {
        Self {
            header: HeapHeader::default(),
            value,
        }
    }

    pub fn header(&self) -> &HeapHeader {
        &self.header
    }

    pub fn header_mut(&mut self) -> &mut HeapHeader {
        &mut self.header
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

#[derive(Debug, Clone)]
struct ArenaSlot<T> {
    generation: u32,
    cell: Option<HeapCell<T>>,
}

#[derive(Debug, Clone)]
pub struct ArenaPage<T: ArenaItem> {
    index: u32,
    capacity: usize,
    slots: Vec<ArenaSlot<T>>,
    free_list: Vec<u32>,
}

impl<T: ArenaItem> ArenaPage<T> {
    pub fn with_capacity(index: u32, capacity: usize) -> Self {
        Self {
            index,
            capacity,
            slots: Vec::with_capacity(capacity),
            free_list: Vec::new(),
        }
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.cell.is_some()).count()
    }

    pub fn is_full(&self) -> bool {
        self.slots.len() >= self.capacity && self.free_list.is_empty()
    }

    pub fn iter_cells(&self) -> impl Iterator<Item = &HeapCell<T>> {
        self.slots.iter().filter_map(|slot| slot.cell.as_ref())
    }

    /// Every live cell with the `GcRef` that addresses it. The sweep needs the
    /// ref, not just the value, to ask whether the mark phase reached it.
    fn iter_refs(&self) -> impl Iterator<Item = (GcRef<T>, &HeapCell<T>)> {
        let page_index = self.index;
        self.slots
            .iter()
            .enumerate()
            .filter_map(move |(slot_index, slot)| {
                let cell = slot.cell.as_ref()?;
                Some((
                    GcRef::new(page_index, slot_index as u32, slot.generation),
                    cell,
                ))
            })
    }

    /// Mark a cell as garbage without reclaiming it (verify mode).
    fn condemn(&mut self, slot_index: u32) {
        if let Some(cell) = self
            .slots
            .get_mut(slot_index as usize)
            .and_then(|slot| slot.cell.as_mut())
        {
            cell.header.condemned = true;
        }
    }

    fn get_cell(&self, gc_ref: GcRef<T>) -> Option<&HeapCell<T>> {
        if gc_ref.arena() != T::ARENA {
            return None;
        }

        let slot = self.slots.get(gc_ref.slot_index() as usize)?;
        if slot.generation != gc_ref.generation() {
            return None;
        }
        slot.cell.as_ref()
    }

    fn get_cell_mut(&mut self, gc_ref: GcRef<T>) -> Option<&mut HeapCell<T>> {
        if gc_ref.arena() != T::ARENA {
            return None;
        }

        let slot = self.slots.get_mut(gc_ref.slot_index() as usize)?;
        if slot.generation != gc_ref.generation() {
            return None;
        }
        slot.cell.as_mut()
    }

    /// Take a specific free slot, for the arena's cross-page free list.
    fn allocate_in_slot(&mut self, slot_index: u32, value: T) -> Option<GcRef<T>> {
        let slot = self.slots.get_mut(slot_index as usize)?;
        if slot.cell.is_some() {
            return None;
        }
        slot.cell = Some(HeapCell::new(value));
        self.free_list.retain(|index| *index != slot_index);
        Some(GcRef::new(self.index, slot_index, slot.generation))
    }

    fn allocate(&mut self, value: T) -> GcRef<T> {
        if let Some(slot_index) = self.free_list.pop() {
            let slot = self
                .slots
                .get_mut(slot_index as usize)
                .expect("free-list slot should exist");
            debug_assert!(slot.cell.is_none());
            slot.cell = Some(HeapCell::new(value));
            return GcRef::new(self.index, slot_index, slot.generation);
        }

        debug_assert!(self.slots.len() < self.capacity);
        let slot_index = self.slots.len() as u32;
        self.slots.push(ArenaSlot {
            generation: 0,
            cell: Some(HeapCell::new(value)),
        });
        GcRef::new(self.index, slot_index, 0)
    }

    fn free_for_gc(&mut self, gc_ref: GcRef<T>) -> bool {
        let Some(slot) = self.slots.get_mut(gc_ref.slot_index() as usize) else {
            return false;
        };
        if slot.generation != gc_ref.generation() {
            return false;
        }
        if slot.cell.take().is_none() {
            return false;
        }

        slot.generation = slot.generation.wrapping_add(1);
        // Deliberately NOT pushed onto `self.free_list`: swept slots are owned
        // by `Arena::free_slots`, which spans pages. Two lists would eventually
        // hand the same slot out twice.
        true
    }
}

#[derive(Debug, Clone)]
pub struct Arena<T: ArenaItem> {
    page_capacity: usize,
    pages: Vec<ArenaPage<T>>,
    len: usize,
    /// Running total of `ArenaItem::footprint` over the live cells.
    bytes: usize,
    /// Slots freed by a sweep, across every page. Without this the arena only
    /// ever reused free slots in its LAST page, so collecting a long-lived page
    /// bought nothing: the next allocation still grew the arena.
    free_slots: Vec<(u32, u32)>,
}

impl<T: ArenaItem> Default for Arena<T> {
    fn default() -> Self {
        Self::with_page_capacity(DEFAULT_PAGE_CAPACITY)
    }
}

impl<T: ArenaItem> Arena<T> {
    pub fn with_page_capacity(page_capacity: usize) -> Self {
        Self {
            page_capacity: page_capacity.max(1),
            pages: Vec::new(),
            len: 0,
            bytes: 0,
            free_slots: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Bytes owned by the live cells, over and above their slots.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn page_capacity(&self) -> usize {
        self.page_capacity
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn pages(&self) -> &[ArenaPage<T>] {
        &self.pages
    }

    pub fn allocate(&mut self, value: T) -> GcRef<T> {
        self.bytes += value.footprint();
        // Reuse whatever the last sweep reclaimed, wherever it lives.
        while let Some((page_index, slot_index)) = self.free_slots.pop() {
            let usable = self
                .pages
                .get(page_index as usize)
                .and_then(|page| page.slots.get(slot_index as usize))
                .is_some_and(|slot| slot.cell.is_none());
            if !usable {
                continue;
            }
            let gc_ref = self.pages[page_index as usize]
                .allocate_in_slot(slot_index, value)
                .expect("slot was just checked to be free");
            self.len += 1;
            return gc_ref;
        }
        if self.pages.last().is_none_or(ArenaPage::is_full) {
            let next_index = self.pages.len() as u32;
            self.pages
                .push(ArenaPage::with_capacity(next_index, self.page_capacity));
        }

        let page = self.pages.last_mut().expect("page should exist");
        let gc_ref = page.allocate(value);
        self.len += 1;
        gc_ref
    }

    pub fn get(&self, gc_ref: GcRef<T>) -> Option<&T> {
        let cell = self.get_cell(gc_ref)?;
        if cell.header.condemned {
            report_condemned_access(gc_ref.raw());
        }
        Some(cell.value())
    }

    pub fn get_mut(&mut self, gc_ref: GcRef<T>) -> Option<&mut T> {
        let cell = self.get_cell_mut(gc_ref)?;
        if cell.header.condemned {
            report_condemned_access(gc_ref.raw());
        }
        Some(cell.value_mut())
    }

    /// Free every live cell the mark phase did not reach. Returns how many went.
    ///
    /// With `condemn_only`, nothing is actually reclaimed — the cells are
    /// flagged instead, so that a later read of one reports a missed root at
    /// the point of use rather than misbehaving somewhere far away.
    pub fn sweep(&mut self, is_live: &dyn Fn(GcRef<T>) -> bool, condemn_only: bool) -> usize {
        let mut doomed: Vec<GcRef<T>> = Vec::new();
        for page in &self.pages {
            for (gc_ref, cell) in page.iter_refs() {
                if !cell.header.condemned && !is_live(gc_ref) {
                    doomed.push(gc_ref);
                }
            }
        }
        for gc_ref in &doomed {
            if condemn_only {
                if let Some(page) = self.pages.get_mut(gc_ref.page_index() as usize) {
                    page.condemn(gc_ref.slot_index());
                }
            } else if self.free_for_gc(*gc_ref) {
                self.free_slots
                    .push((gc_ref.page_index(), gc_ref.slot_index()));
            }
        }
        doomed.len()
    }

    pub fn get_cell(&self, gc_ref: GcRef<T>) -> Option<&HeapCell<T>> {
        if gc_ref.arena() != T::ARENA {
            return None;
        }
        let page = self.pages.get(gc_ref.page_index() as usize)?;
        page.get_cell(gc_ref)
    }

    pub fn get_cell_mut(&mut self, gc_ref: GcRef<T>) -> Option<&mut HeapCell<T>> {
        if gc_ref.arena() != T::ARENA {
            return None;
        }
        let page = self.pages.get_mut(gc_ref.page_index() as usize)?;
        page.get_cell_mut(gc_ref)
    }

    pub fn free_for_gc(&mut self, gc_ref: GcRef<T>) -> bool {
        if gc_ref.arena() != T::ARENA {
            return false;
        }

        let Some(page) = self.pages.get_mut(gc_ref.page_index() as usize) else {
            return false;
        };
        let footprint = page.get_cell(gc_ref).map_or(0, |cell| cell.value().footprint());
        let freed = page.free_for_gc(gc_ref);
        if freed {
            self.len = self.len.saturating_sub(1);
            self.bytes = self.bytes.saturating_sub(footprint);
        }
        freed
    }
}

/// A cell the collector condemned is being read: the collector believed nothing
/// could reach it, and something just did. That means a root was not traced.
/// Reported once per cell so a hot path does not drown the output.
#[cold]
fn report_condemned_access(raw: RawGcRef) {
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<std::collections::HashSet<RawGcRef>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(std::collections::HashSet::new()));
    if let Ok(mut seen) = seen.lock() {
        if !seen.insert(raw) {
            return;
        }
    }
    eprintln!(
        "[gc-verify] a root holding this reference was not traced: {:?} page={} slot={} gen={}",
        raw.arena(),
        raw.page_index(),
        raw.slot_index(),
        raw.generation()
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RootKind {
    StackSlot,
    Register,
    HandleScope,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RootHandle {
    index: usize,
}

impl RootHandle {
    pub const fn index(self) -> usize {
        self.index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RootRecord {
    raw: RawGcRef,
    kind: RootKind,
}

impl RootRecord {
    pub const fn raw(self) -> RawGcRef {
        self.raw
    }

    pub const fn kind(self) -> RootKind {
        self.kind
    }
}

#[derive(Debug, Clone, Default)]
pub struct RootSet {
    slots: Vec<Option<RootRecord>>,
}

impl RootSet {
    pub fn pin<T: ArenaItem>(&mut self, gc_ref: GcRef<T>, kind: RootKind) -> RootHandle {
        let record = RootRecord {
            raw: gc_ref.raw(),
            kind,
        };

        if let Some((index, slot)) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            *slot = Some(record);
            return RootHandle { index };
        }

        let index = self.slots.len();
        self.slots.push(Some(record));
        RootHandle { index }
    }

    pub fn unpin(&mut self, handle: RootHandle) -> Option<RootRecord> {
        self.slots.get_mut(handle.index)?.take()
    }

    pub fn iter(&self) -> impl Iterator<Item = RootRecord> + '_ {
        self.slots.iter().filter_map(|slot| *slot)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Heap {
    strings: Arena<JsString>,
    objects: Arena<JsObject>,
    roots: RootSet,
}

impl Heap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn strings(&self) -> &Arena<JsString> {
        &self.strings
    }

    pub fn strings_mut(&mut self) -> &mut Arena<JsString> {
        &mut self.strings
    }

    pub fn objects(&self) -> &Arena<JsObject> {
        &self.objects
    }

    pub fn objects_mut(&mut self) -> &mut Arena<JsObject> {
        &mut self.objects
    }

    pub fn roots(&self) -> &RootSet {
        &self.roots
    }

    pub fn roots_mut(&mut self) -> &mut RootSet {
        &mut self.roots
    }

    pub fn allocate_string(&mut self, string: JsString) -> GcRef<JsString> {
        self.strings.allocate(string)
    }

    pub fn allocate_object(&mut self, object: JsObject) -> GcRef<JsObject> {
        self.objects.allocate(object)
    }
}
