fn main() {
    let result = ureq::get("not a url").call();
    println!("{:?}", result);
}
