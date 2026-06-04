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
}

impl ArenaItem for JsString {
    const ARENA: HeapArena = HeapArena::String;
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
}

impl Default for HeapHeader {
    fn default() -> Self {
        Self {
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
        self.free_list.push(gc_ref.slot_index());
        true
    }
}

#[derive(Debug, Clone)]
pub struct Arena<T: ArenaItem> {
    page_capacity: usize,
    pages: Vec<ArenaPage<T>>,
    len: usize,
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
        }
    }

    pub fn len(&self) -> usize {
        self.len
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
        // Phase 7: freed slots in earlier pages are never reused here; mark-sweep
        // will need a cross-page free scan or a separate global free list.
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
        self.get_cell(gc_ref).map(HeapCell::value)
    }

    pub fn get_mut(&mut self, gc_ref: GcRef<T>) -> Option<&mut T> {
        self.get_cell_mut(gc_ref).map(HeapCell::value_mut)
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
        let freed = page.free_for_gc(gc_ref);
        if freed {
            self.len = self.len.saturating_sub(1);
        }
        freed
    }
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
