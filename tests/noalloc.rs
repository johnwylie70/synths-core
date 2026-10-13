//! Nothing may ask for memory, or give it back, while sound is being made.
//! This test puts a counter in front of the allocator and renders with
//! everything going: notes, the live input, every voice effect, recording.

mod common;
use common::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use synths_core::abi::*;
use synths_core::event::{ALL_PARTS, RECORD_INPUT, RECORD_MASTER, RECORD_PART};
use synths_core::input_abi::*;
use synths_core::params::*;
use synths_core::{Engine, Event};

thread_local! {
    /// How many times this thread has asked the allocator for anything.
    static CALLS: Cell<u64> = const { Cell::new(0) };
    /// How many bytes it has asked for.
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        CALLS.with(|c| c.set(c.get() + 1));
        BYTES.with(|c| c.set(c.get() + layout.size() as u64));
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CALLS.with(|c| c.set(c.get() + 1));
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        CALLS.with(|c| c.set(c.get() + 1));
        System.realloc(ptr, layout, size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn calls() -> u64 {
    CALLS.with(|c| c.get())
}

#[test]
fn the_counter_counts() {
    let before = calls();
    drop(std::hint::black_box(vec![0u8; 100]));
    assert!(calls() >= before + 2);
}

#[test]
fn an_engine_is_about_as_big_as_the_document_says() {
    // "About 2 MB at 48 kHz": the echo's two seconds are half of it.
    let before = BYTES.with(|c| c.get());
    let e = Engine::new(SR);
    let bytes = BYTES.with(|c| c.get()) - before;
    println!("an engine at 48 kHz asks for {bytes} bytes");
    assert!(bytes > 1_500_000 && bytes < 2_500_000, "{bytes}");
    drop(e);
}

#[test]
fn rendering_with_everything_on_never_touches_the_allocator() {
    // Setting up may allocate: the engine, somewhere to record, the input.
    let mut e = Engine::new(SR);
    for slot in 0..3 {
        e.record_buffer(slot, 30000, 1 + (slot % 2) as u32).unwrap();
    }
    let x = speech::speech(SR, 3.0);
    let y = hiss(4, 0.3, x.len());
    let (mut l, mut r) = (vec![0.0f32; 4096], vec![0.0f32; 4096]);
    let events = |e: &mut Engine, at: f64| {
        let mut all_taken = true;
        let mut push = |ev: Event| all_taken &= e.push(ev);
        for part in 0..12u16 {
            push(Event::param(at, part, FX_PITCH, part as f32 - 6.0, 0.05));
            push(Event::param(at + 50.0, part, FX_COMB, 0.5, 0.0));
            push(Event::param(at + 99.0, part, FX_RING, 0.5, 0.0));
            push(Event::note_on(at + 100.0, part, 1 + part as u32, 40.0 + part as f32, 0.7));
            push(Event::note_off(at + 9000.0, part, 1 + part as u32, 0.0));
            push(Event::param(at + 20000.0, part, FX_PITCH, 0.0, 0.0));
        }
        for (i, source) in [RECORD_INPUT, RECORD_PART, RECORD_MASTER].into_iter().enumerate() {
            push(Event::record_start(at + 1000.0 + 15000.0 * i as f64, i as u32, source, 31));
            push(Event::record_stop(at + 14000.0 + 15000.0 * i as f64));
        }
        push(Event::param(at, ALL_PARTS, SEND_REVERB, 0.3, 0.0));
        push(Event::param(at, ALL_PARTS, SEND_DELAY, 0.3, 0.0));
        push(Event::param(at, 31, DRIVE, 0.4, 0.0));
        push(Event::param(at, 31, FILTER_TYPE, 3.0, 0.0));
        push(Event::param(at, 0, INPUT_GATE, -40.0, 0.0));
        push(Event::param(at + 30000.0, 0, INPUT_PART, 3.0, 0.0));
        push(Event::param(at + 40000.0, 0, INPUT_MONITOR, 0.0, 0.0));
        assert!(all_taken);
    };

    let before = calls();
    events(&mut e, 0.0);
    let mut at = 0;
    for block in [128usize, 1, 4096, 77, 128, 128, 2048].into_iter().cycle() {
        if at + block > x.len() {
            break;
        }
        let stereo = (at / 4096) % 2 == 1;
        e.render_io(&x[at..at + block], if stereo { Some(&y[at..at + block]) } else { None }, &mut l[..block], &mut r[..block]);
        at += block;
        if at % 3 == 0 {
            e.render(&mut l[..block], &mut r[..block]);
        }
        std::hint::black_box(e.recording());
    }
    let after = calls();
    assert_eq!(after - before, 0, "the allocator was called {} times during rendering", after - before);
    // It did all make sound, and all three recordings were made.
    assert!(peak(&l) > 0.0 && (0..3).all(|slot| e.sample_frames(slot) == 13000));

    // The same through the C interface: filling the buffers, pushing events and rendering.
    unsafe {
        let engine = synths_create(SR);
        assert_eq!(synths_record_buffer(engine, 0, 48000, 2), 0);
        let mut words = [0u32; 4];
        let before = calls();
        let buffer = synths_event_buffer(engine) as *mut Event;
        buffer.write(Event::param(0.0, 31, FX_PITCH, 7.0, 0.0));
        buffer.add(1).write(Event::param(0.0, 31, FX_COMB, 0.5, 0.0));
        buffer.add(2).write(Event::record_start(64.0, 0, RECORD_PART, 31));
        buffer.add(3).write(Event::note_on(0.0, 0, 0, 60.0, 1.0));
        assert_eq!(synths_push_events(engine, 4), 4);
        for block in 0..200 {
            core::ptr::copy_nonoverlapping(x[block * 128..].as_ptr(), synths_input_left(engine), 128);
            core::ptr::copy_nonoverlapping(y[block * 128..].as_ptr(), synths_input_right(engine), 128);
            assert_eq!(synths_render_io(engine, 128, 1 + (block % 2) as u32), 128);
            synths_render_io_into(engine, x[block * 128..].as_ptr(), core::ptr::null(), l.as_mut_ptr(), r.as_mut_ptr(), 128);
            synths_record_read(engine, words.as_mut_ptr());
        }
        let after = calls();
        assert_eq!(after - before, 0, "the allocator was called {} times through the C interface", after - before);
        assert_eq!((words, synths_sample_frames(engine, 0)), ([0, 0, 48000, 48000], 48000));
        synths_destroy(engine);
    }
}
