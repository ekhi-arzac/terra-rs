use bytes::BytesMut;
use std::{hint::black_box, time::Instant};
use terraria_protocol::{Equipment, Frame, Packet};

fn main() {
    let packet = Packet::Equipment(Equipment {
        player: 0,
        slot: 51,
        stack: 99,
        prefix: 0,
        item: 1,
        flags: 0,
    });
    let start = Instant::now();
    let iterations = 1_000_000;
    for _ in 0..iterations {
        let bytes = black_box(&packet).encode().unwrap();
        let mut input = BytesMut::from(bytes);
        let frame = Frame::split(&mut input).unwrap().unwrap();
        black_box(Packet::decode(&frame).unwrap());
    }
    println!(
        "Equipment encode + frame + decode: {:.1} ns/packet",
        start.elapsed().as_nanos() as f64 / f64::from(iterations)
    );
}
