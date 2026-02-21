//! Criterion benchmark: parse 1000 messages repeatedly and report messages per second.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use parser::{Parser, StateMachineParser};
use protocol::Message;

fn make_messages(message_count: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..message_count {
        let data_len = i % 64;
        let data: Vec<u8> = vec![1; data_len];
        let msg = Message::builder()
            .address((i % 256) as u8)
            .destination((i.wrapping_mul(3) % 256) as u8)
            .data(data)
            .build()
            .expect("message build");
        msg.write_bytes(&mut out).expect("write");
    }
    out
}

fn bench_parse_1000_messages(c: &mut Criterion) {
    const MESSAGE_COUNT: usize = 1000;
    let buffer = make_messages(MESSAGE_COUNT);

    let mut group = c.benchmark_group("StateMachineParser");
    group.throughput(Throughput::Elements(MESSAGE_COUNT as u64));

    group.bench_function("parse_1000_messages", |b| {
        b.iter(|| {
            let mut parser = StateMachineParser::new();
            let results = parser.feed(std::hint::black_box(&buffer));
            assert_eq!(results.len(), 1000, "expected 1000 complete messages");
        });
    });
    group.finish();
}

criterion_group!(benches, bench_parse_1000_messages);
criterion_main!(benches);
