fn main() {
    let source = "my_qos = QoSReliabilityPolicy.BEST_EFFORT";
    let ast = rustpython_parser::parse(source, rustpython_parser::Mode::Module, "<embedded>");
    println!("{:?}", ast);
}
