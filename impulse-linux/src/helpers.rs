extern "C" {
    fn impulse_init_webengine();
}

pub fn init_webengine() {
    unsafe {
        impulse_init_webengine();
    }
}
