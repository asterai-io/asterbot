use crate::bindings::exports::asterbot::core::core::Guest;

#[allow(warnings)]
mod bindings;

struct Component;

impl Guest for Component {
    fn converse(input: String) -> String {
        String::new()
    }
}

bindings::export!(Component with_types_in bindings);
