/// `ServerLoginPacketListenerImpl`'s ordered login transitions.
#[derive(Default, Debug, PartialEq, Eq)]
pub enum LoginState {
    #[default]
    Hello,
    Key,
    Authenticating,
    Proxy,
    Verifying,
    ProtocolSwitching,
    Configuration,
}

impl LoginState {
    pub fn acknowledge(&mut self, authenticated: bool) -> bool {
        if *self != Self::ProtocolSwitching || !authenticated {
            return false;
        }
        *self = Self::Configuration;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::LoginState;

    #[test]
    fn acknowledgement_requires_success_and_authenticated_profile() {
        for mut state in [
            LoginState::Hello,
            LoginState::Key,
            LoginState::Authenticating,
            LoginState::Proxy,
            LoginState::Verifying,
        ] {
            assert!(!state.acknowledge(true));
        }
        let mut state = LoginState::ProtocolSwitching;
        assert!(!state.acknowledge(false));
        assert!(state.acknowledge(true));
        assert!(!state.acknowledge(true));
    }
}
