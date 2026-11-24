use dsp::FramesPerTick;
use xtask::price::{
    Block, BlockPrice, ListingErrorKind, ProxyCycles, Share, Verdict, price_listing,
};

fn only_price(listing: &str) -> BlockPrice {
    let prices = price_listing(listing).expect("the listing parses");
    assert_eq!(prices.len(), 1, "one block in the listing");
    prices[0]
}

fn cycles_of(listing: &str) -> u32 {
    only_price(listing).cycles.get()
}

fn nops(block: &str, frames: u32, count: u32) -> String {
    let mut listing = format!("block {block} frames {frames}\n");
    for index in 0..count {
        listing.push_str(&format!("{:x}: bf00 nop\n", index * 2));
    }
    listing
}

#[test]
fn a_straight_line_block_under_its_share_passes_with_its_cycle_figure() {
    let listing = "\
block capture-ramp frames 6
       0: b5f0         \tpush\t{r4, r5, r6, r7, lr}
       2: e9d0 2300    \tldrd\tr2, r3, [r0]
       6: fb82 2301    \tsmull\tr2, r3, r2, r1
       a: e9c0 2300    \tstrd\tr2, r3, [r0]
       e: bdf0         \tpop\t{r4, r5, r6, r7, pc}
";
    let price = only_price(listing);
    assert_eq!(price.block, Block::CaptureRamp);
    assert_eq!(price.frames, FramesPerTick::Six);
    assert_eq!(price.instructions.count(), 5);
    assert_eq!(price.cycles, ProxyCycles::new(21));
    assert_eq!(price.verdict(), Verdict::WithinShare(ProxyCycles::new(278)));
}

#[test]
fn a_branch_costs_two_when_the_next_line_is_its_target_and_one_when_it_falls_through() {
    let loop_twice = "\
block capture-ramp frames 6
10: 3901 subs r1, #1
12: d1fd bne 0x10
10: 3901 subs r1, #1
12: d1fd bne 0x10
14: 4770 bx lr
";
    assert_eq!(cycles_of(loop_twice), 1 + 2 + 1 + 1 + 2);
}

#[test]
fn the_last_instruction_of_a_path_leaves_it_so_a_final_branch_is_taken() {
    assert_eq!(
        cycles_of("block dma-moves frames 6\n0: f000 b800 b.w 0x400\n"),
        2
    );
}

#[test]
fn a_call_out_of_the_block_is_taken_even_when_the_block_resumes_at_its_return_address() {
    let call_then_return = "\
block dispatch-fade frames 12
0: f000 f800 bl 0x200
4: 2000 movs r0, #0
6: f000 b800 b.w 0xa
a: 4770 bx lr
";
    assert_eq!(cycles_of(call_then_return), 2 + 1 + 2 + 2);
}

#[test]
fn register_lists_cost_one_plus_each_register_and_loading_pc_adds_the_refill() {
    let listing = "\
block bypass-out frames 12
0: e92d 4ff0 push.w {r4, r5, r6, r7, r8, r9, r10, r11, lr}
4: e8bd 0f00 pop.w {r8-r11}
8: c80e ldm r0!, {r1, r2, r3}
a: bdf0 pop {r4, r5, r6, r7, pc}
";
    assert_eq!(cycles_of(listing), 10 + 5 + 4 + 8);
}

#[test]
fn every_load_and_store_width_costs_two_and_a_double_word_costs_three() {
    let listing = "\
block bypass-out frames 12
0: 7808 ldrb r0, [r1]
2: f8b1 0002 ldrh.w r0, [r1, #2]
6: e851 0f00 ldrex r0, [r1]
a: e841 2300 strex r3, r2, [r1]
e: bf18 it ne
10: 6808 ldrne r0, [r1]
12: e9d1 2300 ldrd r2, r3, [r1]
16: e9c1 2300 strd r2, r3, [r1]
";
    assert_eq!(cycles_of(listing), 2 + 2 + 2 + 2 + 1 + 2 + 3 + 3);
}

#[test]
fn conditional_and_compare_branches_are_branches_but_bit_clear_and_bit_insert_are_not() {
    let listing = "\
block bypass-out frames 12
0: d900 bls 0x4
4: f200 8000 bhi.w 0x100
100: b108 cbz r0, 0x106
102: ea21 0002 bic.w r0, r1, r2
106: f361 0007 bfi r0, r1, #0, #8
10a: e8df f000 tbb [pc, r0]
";
    assert_eq!(cycles_of(listing), 2 + 2 + 1 + 1 + 1 + 2);
}

#[test]
fn floating_point_follows_the_m4_table_of_the_research_note() {
    let listing = "\
block fade-out frames 24
0: ee20 0a01 vmul.f32 s0, s0, s2
4: eea0 0a01 vfma.f32 s0, s0, s2
8: ee80 0a01 vdiv.f32 s0, s0, s2
c: ed90 0a00 vldr s0, [r0]
10: ed2d 8b04 vpush {d8, d9}
";
    assert_eq!(cycles_of(listing), 1 + 3 + 14 + 2 + 3);
}

#[test]
fn a_division_has_no_proxy_price_and_the_listing_is_refused_at_its_line() {
    let listing = "block dispatch-on frames 6\n0: fb90 f0f1 sdiv r0, r0, r1\n";
    let error = price_listing(listing).expect_err("division has no proxy price");
    assert_eq!(error.line, 2);
    assert_eq!(
        error.kind,
        ListingErrorKind::NoProxyPrice("sdiv".to_string())
    );
}

#[test]
fn a_block_that_d4_turns_off_at_twenty_four_frames_fails_whatever_its_count() {
    let price = only_price("block on-peq frames 24\n0: 4770 bx lr\n");
    assert_eq!(price.verdict(), Verdict::OffAtThisFrameCount);
    assert!(!price.passes());
}

#[test]
fn every_d3_block_passes_at_exactly_its_share_and_fails_one_cycle_over() {
    let frame_counts = [
        FramesPerTick::Six,
        FramesPerTick::Twelve,
        FramesPerTick::TwentyFour,
    ];
    let d3 = Block::ALL.map(|block| {
        (
            block.id(),
            frame_counts.map(|frames| match block.share(frames) {
                Share::Cap(cycles) => cycles.get(),
                Share::OffAtThisFrameCount => 0,
            }),
        )
    });
    for (block, shares) in d3 {
        for (frames, share) in [6, 12, 24].into_iter().zip(shares) {
            let label = format!("{block} at {frames} frames");
            if share == 0 {
                let off = only_price(&nops(block, frames, 1));
                assert_eq!(off.verdict(), Verdict::OffAtThisFrameCount, "{label}");
                continue;
            }
            let at_share = only_price(&nops(block, frames, share));
            assert_eq!(
                at_share.verdict(),
                Verdict::WithinShare(ProxyCycles::new(share)),
                "{label}"
            );
            let over = only_price(&nops(block, frames, share + 1));
            assert_eq!(
                over.verdict(),
                Verdict::OverShare(ProxyCycles::new(share)),
                "{label}"
            );
        }
    }
}

#[test]
fn the_report_gives_proxy_times_one_and_a_half_rounded_half_up_without_gating_it() {
    let worst_twelve_frame_tick = only_price(&nops("dsp-worst-tick", 12, 5409));
    assert!(worst_twelve_frame_tick.passes());
    assert_eq!(
        worst_twelve_frame_tick.to_string(),
        "PASS dsp-worst-tick at 12 frames: 5,409 instructions, 5,409 proxy cycles, 43.3% of the \
         12,500-cycle tick, within its budget share of 6,250 (50.0%). Proxy x1.5 is 8,114 (64.9%), \
         reported, not gated. Cortex-M4 proxy prices until measured on hardware."
    );
}

#[test]
fn blocks_in_one_listing_are_priced_apart_and_a_branch_never_flows_into_the_next_block() {
    let listing = "\
block capture-ramp frames 12
0: d1fe bne 0x0
block usb-interrupt frames 12
2: bf00 nop
";
    let prices = price_listing(listing).expect("the listing parses");
    let summary: Vec<(Block, u32)> = prices.iter().map(|p| (p.block, p.cycles.get())).collect();
    assert_eq!(summary, [(Block::CaptureRamp, 2), (Block::UsbInterrupt, 1)]);
}

#[test]
fn malformed_listings_are_refused_with_the_line_and_the_reason() {
    let cases = [
        (
            "block on-peq frames 6\n       0: \tpush\t{r4, lr}\n",
            2,
            ListingErrorKind::MissingEncoding,
        ),
        (
            "0: bf00 nop\n",
            1,
            ListingErrorKind::InstructionBeforeAnyBlock,
        ),
        (
            "block reverb frames 6\n0: bf00 nop\n",
            1,
            ListingErrorKind::UnknownBlock("reverb".to_string()),
        ),
        (
            "block on-peq frames 8\n0: bf00 nop\n",
            1,
            ListingErrorKind::UnknownFrameCount("8".to_string()),
        ),
        (
            "block on-peq 6\n",
            1,
            ListingErrorKind::MalformedBlockHeader,
        ),
        (
            "block on-peq frames 6\n0: 4000 0000 .word 0x40000000\n",
            2,
            ListingErrorKind::DataInPath(".word".to_string()),
        ),
        (
            "block on-peq frames 6\nblock fade-out frames 6\n0: bf00 nop\n",
            1,
            ListingErrorKind::EmptyBlock("on-peq"),
        ),
        (
            "block on-peq frames 6\nthis is not objdump\n",
            2,
            ListingErrorKind::UnreadableLine,
        ),
    ];
    for (listing, line, kind) in cases {
        let error = price_listing(listing).expect_err(listing);
        assert_eq!((error.line, error.kind), (line, kind), "{listing}");
    }
}
