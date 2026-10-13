use std::{io, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    task::JoinHandle,
};

const CHILD_TEST_PARENT_TIMEOUT: Duration = Duration::from_secs(45);

// The outer deadline also covers a child whose runtime thread cannot poll its own timer.
pub async fn run_isolated_child_test(test_name: &str, child_env: &str) -> Result<(), String> {
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => return Err(format!("failed to locate test executable: {error}")),
    };
    let mut child = match Command::new(executable)
        .args([test_name, "--exact", "--nocapture"])
        .env(child_env, "1")
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return Err(format!(
                "failed to spawn isolated regression child: {error}"
            ));
        }
    };
    let Some(stdout) = child.stdout.take() else {
        return Err("isolated regression child stdout was not piped".to_owned());
    };
    let Some(stderr) = child.stderr.take() else {
        return Err("isolated regression child stderr was not piped".to_owned());
    };
    let stdout = tokio::spawn(read_pipe(stdout));
    let stderr = tokio::spawn(read_pipe(stderr));

    let status = match tokio::time::timeout(CHILD_TEST_PARENT_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            return Err(stop_child_and_capture(
                &mut child,
                stdout,
                stderr,
                format!("failed to wait for regression child: {error}"),
            )
            .await);
        }
        Err(_) => {
            return Err(stop_child_and_capture(
                &mut child,
                stdout,
                stderr,
                format!(
                    "regression child exceeded the {}-second parent deadline",
                    CHILD_TEST_PARENT_TIMEOUT.as_secs()
                ),
            )
            .await);
        }
    };

    let stdout = collect_pipe(stdout).await;
    let stderr = collect_pipe(stderr).await;
    if !status.success() {
        return Err(format!(
            "child test failed with {status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    if !stdout.contains("test result: ok. 1 passed;") {
        return Err(format!(
            "child did not run exactly one passing test\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    Ok(())
}

async fn stop_child_and_capture(
    child: &mut Child,
    stdout: JoinHandle<io::Result<Vec<u8>>>,
    stderr: JoinHandle<io::Result<Vec<u8>>>,
    reason: String,
) -> String {
    let kill_result = child.start_kill().map_err(|error| error.to_string());
    let reap_result = child.wait().await.map_err(|error| error.to_string());
    let stdout = collect_pipe(stdout).await;
    let stderr = collect_pipe(stderr).await;
    format!(
        "{reason}\nkill result: {kill_result:?}\nreap result: {reap_result:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    )
}

async fn read_pipe<R: AsyncRead + Unpin>(mut reader: R) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader.read_to_end(&mut output).await?;
    Ok(output)
}

async fn collect_pipe(reader: JoinHandle<io::Result<Vec<u8>>>) -> String {
    match reader.await {
        Ok(Ok(output)) => String::from_utf8_lossy(&output).into_owned(),
        Ok(Err(error)) => format!("(failed to read child output: {error})"),
        Err(error) => format!("(child output reader failed: {error})"),
    }
}
