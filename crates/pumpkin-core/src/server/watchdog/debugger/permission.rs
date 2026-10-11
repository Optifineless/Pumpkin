use std::io;

#[cfg(target_os = "linux")]
pub(super) fn yama_scope() -> io::Result<Option<u8>> {
    match std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope") {
        Ok(scope) => scope.trim().parse().map(Some).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

// PR_SET_PTRACER is a Yama mode-1 exception, not a prerequisite for ptrace.
pub(super) fn with_permission<T>(
    scope: Option<u8>,
    mut permission: impl FnMut(bool) -> io::Result<()>,
    capture: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    match scope {
        Some(1) => {
            permission(true)?;
            let result = capture();
            permission(false)?;
            result
        }
        // Modes 2/3 still enforce kernel policy; the debugger must report failure.
        None | Some(0 | 2 | 3) => capture(),
        Some(_) => Err(io::Error::other("Unknown Yama ptrace scope")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn yama_scope_controls_authorization_and_preserves_failures() {
        for scope in [None, Some(0), Some(1), Some(2), Some(3)] {
            let calls = RefCell::new(Vec::new());
            let result = with_permission(
                scope,
                |allow| {
                    calls
                        .borrow_mut()
                        .push(if allow { "allow" } else { "revoke" });
                    if scope != Some(1) {
                        return Err(io::Error::from(io::ErrorKind::Unsupported));
                    }
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("capture");
                    if matches!(scope, Some(2 | 3)) {
                        Err(io::Error::from(io::ErrorKind::PermissionDenied))
                    } else {
                        Ok(())
                    }
                },
            );
            if scope == Some(1) {
                assert_eq!(*calls.borrow(), ["allow", "capture", "revoke"]);
            } else {
                assert_eq!(*calls.borrow(), ["capture"]);
            }
            assert_eq!(result.is_ok(), !matches!(scope, Some(2 | 3)));
        }
        let captured = RefCell::new(false);
        assert!(
            with_permission(
                Some(1),
                |_| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                || {
                    *captured.borrow_mut() = true;
                    Ok(())
                }
            )
            .is_err()
        );
        assert!(!*captured.borrow());
        let calls = RefCell::new(Vec::new());
        let result: io::Result<()> = with_permission(
            Some(1),
            |allow| {
                calls.borrow_mut().push(allow);
                Ok(())
            },
            || Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        );
        assert_eq!(*calls.borrow(), [true, false]);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(
            with_permission(
                Some(1),
                |allow| if allow {
                    Ok(())
                } else {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                },
                || Ok(())
            )
            .is_err()
        );
    }
}
