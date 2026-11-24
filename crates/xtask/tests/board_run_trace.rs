use xtask::board_run::{
    Ending, Entry, Event, I2cStatus, I2cTransaction, Level, Nanoseconds, Trace,
};

#[test]
fn a_model_trace_reads_into_timed_events_in_time_order() {
    let trace = Trace::parse(
        "0 behavior idle-skip guess | a poll jumps time\n\
         9431750 preamp-word bits=24 value=0x000000\n\
         9416750 pin P1_4 low\n\
         937536030 i2c write 0x49 0x10,0xff ack\n\
         938000000 i2c read 0x49 reg 0x71 value 0x00 nack-address\n\
         939000000 i2c address 0x13 nack-data@1\n\
         1053437185 breadcrumb-magic 0x4d324243\n\
         1053437190 breadcrumb 0xa8b040a\n\
         1501431670 end time-limit\n",
    )
    .expect("the trace reads");
    let events: Vec<&Event> = trace.entries.iter().map(|entry| &entry.event).collect();
    assert_eq!(
        trace.entries[1],
        Entry {
            at: Nanoseconds(9_416_750),
            event: Event::Pin {
                port: 1,
                number: 4,
                level: Level::Low
            }
        }
    );
    assert_eq!(events[2], &Event::PreampWord { bits: 24, value: 0 });
    assert_eq!(
        events[3],
        &Event::I2c(I2cTransaction::Write {
            address: 0x49,
            bytes: vec![0x10, 0xFF],
            status: I2cStatus::Acked
        })
    );
    assert_eq!(
        events[4],
        &Event::I2c(I2cTransaction::Read {
            address: 0x49,
            register: 0x71,
            value: 0,
            status: I2cStatus::NackAddress
        })
    );
    assert_eq!(
        events[5],
        &Event::I2c(I2cTransaction::AddressOnly {
            address: 0x13,
            status: I2cStatus::NackData(1)
        })
    );
    assert_eq!(events[6], &Event::BreadcrumbMagic(0x4D32_4243));
    assert_eq!(events[7], &Event::Breadcrumb(0x0A8B_040A));
    assert_eq!(events[8], &Event::End(Ending::TimeLimit));
    assert_eq!(
        trace.guesses(),
        vec!["idle-skip: a poll jumps time".to_string()]
    );
}

#[test]
fn a_line_the_gate_does_not_know_is_refused_rather_than_skipped() {
    let error = Trace::parse("5 teleport 0x1\n").expect_err("an unknown kind is refused");
    assert!(error.to_string().contains("teleport"), "{error}");
}
