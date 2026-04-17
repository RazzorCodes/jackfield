fn main() {
    tonic_build::compile_protos("proto/jackfield.proto").unwrap();
}
