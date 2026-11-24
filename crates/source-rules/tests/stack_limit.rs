use std::fs;
use std::path::PathBuf;

fn firmware(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../board/src/firmware")
        .join(file);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn the_hand_over_quiesce_lets_hardfault_push_past_the_stack_limit() {
    let map = firmware("map.rs");
    assert!(
        map.contains("pub const SCB_CCR_STKOFHFNMIGN: u32 = 1 << 10;"),
        "CCR.STKOFHFNMIGN is bit 10"
    );
    let hand_over = firmware("hand_over.rs");
    let table = hand_over
        .split("pub static HAND_OVER_WRITES")
        .nth(1)
        .and_then(|rest| rest.split("END_OF_WRITES").next())
        .expect("the hand-over table is in hand_over.rs");
    assert!(
        table.contains("modify(SCB_CCR, !SCB_CCR_STKOFHFNMIGN, SCB_CCR_STKOFHFNMIGN)"),
        "the register-only quiesce sets CCR.STKOFHFNMIGN before any code can use the stack"
    );
}
