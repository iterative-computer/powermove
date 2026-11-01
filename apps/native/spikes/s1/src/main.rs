fn main() {
    let platform = v8::new_default_platform(0, false).make_shared();
    v8::V8::initialize_platform(platform);
    v8::V8::initialize();
    assert!(v8::V8::is_sandbox_enabled());
    println!("{{\"sandbox\":true,\"v8\":\"{}\"}}", v8::V8::get_version());
    unsafe { v8::V8::dispose(); }
    v8::V8::dispose_platform();
}
