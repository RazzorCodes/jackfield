fn main() {
    #[cfg(feature = "grpc")]
    tonic_build::configure()
        .compile_protos(&["proto/jackfield.proto"], &["proto/"])
        .unwrap();

    #[cfg(all(feature = "websocket", not(feature = "grpc")))]
    prost_build::compile_protos(&["proto/jackfield.proto"], &["proto/"]).unwrap();
}
