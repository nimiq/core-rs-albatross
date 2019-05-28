use collections::bitset::BitSet;
use primitives::validators::{Slot, Slots};

pub struct SlashedSlots<'a> {
    slots: &'a Slots,
    slashed_set: &'a BitSet,
}

impl<'a> SlashedSlots<'a> {

    pub fn new(slots: &'a Slots, slashed_set: &'a BitSet) -> Self {
        Self {
            slots,
            slashed_set,
        }
    }

    pub fn slot_states(&self) -> impl Iterator<Item=(&Slot, bool)> {
        self.slots.iter().zip(self.slashed_set.iter_bits().map(|b| !b))
    }

    pub fn enabled(&self) -> impl Iterator<Item=&Slot> {
        self.slot_states()
            .filter_map(|(slot, ok)| if ok { Some(slot) } else { None })
    }

    pub fn slashed(&self) -> impl Iterator<Item=&Slot> {
        self.slot_states()
            .filter_map(|(slot, ok)| if ok { None } else { Some(slot) })
    }


}
