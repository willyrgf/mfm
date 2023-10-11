#[derive(Debug)]
enum States<T: StateHandler> {
    Setup(T),
    Report(T),
}

trait StateHandler {
    fn handler(&self);
}

struct SetupState;
impl StateHandler for SetupState {
    fn handler(&self) {
        println!("handling setup state")
    }
}

struct ReportState;
impl StateHandler for ReportState {
    fn handler(&self) {
        println!("handling report state")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn setup_state_initialization() {
        let state: States<SetupState> = States::Setup(SetupState);
        match state {
            States::Setup(t) => t.handler(),
            _ => panic!("expected Setup state"),
        }
    }
}
