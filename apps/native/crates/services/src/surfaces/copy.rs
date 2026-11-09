//! Non-visual copy projections from Opus's surface specs at ce36cb6fc/78294c4c9.
//! These are strings/facts only; native hosts own controls, tokens and focus.
use super::{
    agent_auth::SignInStep,
    publishing::{Phase, Problem, SandboxState},
    transcription::JobState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub title: String,
    pub line: String,
    pub action: Option<String>,
}
fn message(title: impl Into<String>, line: impl Into<String>, action: Option<&str>) -> Message {
    Message {
        title: title.into(),
        line: line.into(),
        action: action.map(String::from),
    }
}
pub fn sign_in(step: SignInStep, signup: bool, provider: &str, email: &str) -> Option<Message> {
    match step {
        SignInStep::Start => Some(message(
            if signup {
                "Create Your Powermove Account"
            } else {
                "Sign In to Powermove"
            },
            if signup {
                "Everything you publish to the Store lives under your handle."
            } else {
                "Publish and manage your extensions on the Store."
            },
            Some("Continue with Email"),
        )),
        SignInStep::Browser => Some(message(
            "Finish Signing In in Your Browser",
            format!(
                "Powermove opened {provider} in your browser. When you’re done there, you’ll come straight back here."
            ),
            Some("Cancel"),
        )),
        SignInStep::Code => Some(message(
            "Check Your Email",
            format!("Enter the 6-digit code sent to {email}."),
            None,
        )),
        SignInStep::Handle => Some(message(
            "Choose Your Handle",
            "It’s your name on the Store. Extensions you publish live under it, and it can’t be changed later.",
            Some("Choose Handle"),
        )),
        _ => None,
    }
}
#[derive(Debug, Clone, Copy)]
pub enum AuthError {
    Browser,
    EmailRejected,
    Offline,
    RateLimited,
    OldClient,
    ReturnLink,
    WrongCode,
    ExpiredCode,
    CheckCode,
    InvalidHandle,
    HandleTaken,
    HandleReserved,
    SaveHandle,
}
pub fn auth_error(error: AuthError, provider: &str, handle: &str) -> String {
    match error {
        AuthError::Browser => format!("Couldn’t open {provider}. Try again."),
        AuthError::EmailRejected => {
            "Couldn’t send a code to this address. Check it and try again.".into()
        }
        AuthError::Offline => "Couldn’t send a code. Check your connection and try again.".into(),
        AuthError::RateLimited => "Too many attempts. Wait a minute, then try again.".into(),
        AuthError::OldClient => {
            "This version of Powermove is too old to sign in. Update Powermove, then try again."
                .into()
        }
        AuthError::ReturnLink => "Couldn’t finish signing in. Try again.".into(),
        AuthError::WrongCode => "That code didn’t match. Check it, or send a new one.".into(),
        AuthError::ExpiredCode => "That code has expired. Send a new one.".into(),
        AuthError::CheckCode => {
            "Couldn’t check the code. Check your connection and try again.".into()
        }
        AuthError::InvalidHandle => {
            "Use 2 to 39 characters, starting with a letter or number.".into()
        }
        AuthError::HandleTaken => format!("@{handle} is taken. Try another."),
        AuthError::HandleReserved => format!("@{handle} is reserved. Try another."),
        AuthError::SaveHandle => {
            "Couldn’t save your handle. Check your connection and try again.".into()
        }
    }
}
pub fn resend(seconds: u64) -> String {
    if seconds == 0 {
        "Resend Code".into()
    } else {
        format!("Resend in {seconds} s")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderState {
    Checking,
    Disconnected,
    Connecting,
    Reconnecting,
    ApiUnset,
    Unavailable,
}
pub fn connection(state: ProviderState, provider: &str) -> Message {
    match state {
        ProviderState::Checking => message(
            "Checking Connection…",
            format!("Checking your {provider} connection."),
            Some("Checking…"),
        ),
        ProviderState::Disconnected => message(
            format!("Connect {provider}"),
            format!("Use your {provider} subscription to power the Powermove agent."),
            Some(&format!("Connect {provider}")),
        ),
        ProviderState::Connecting => message(
            format!("Finish Connecting {provider}"),
            "Finish signing in in your browser.",
            Some("Waiting for Sign-In…"),
        ),
        ProviderState::Reconnecting => message(
            format!("Finish Connecting {provider}"),
            "Connection interrupted. Reconnecting…",
            Some("Waiting for Sign-In…"),
        ),
        ProviderState::ApiUnset => message(
            "Bring Your Own Model",
            "Use an API, Ollama, or LM Studio. Set up your model in Settings.",
            Some("Open Settings"),
        ),
        ProviderState::Unavailable => message(
            format!("{provider} Couldn’t Start"),
            format!("Powermove couldn’t start its {provider} service."),
            Some("Try Again"),
        ),
    }
}
#[derive(Debug, Clone, Copy)]
pub enum RuntimeState {
    Needed,
    Downloading,
    Activating,
    ChecksumMismatch,
    Offline,
    RegistryProblem,
    BackgroundFailure,
}
pub fn runtime(state: RuntimeState, runtime: &str) -> Option<Message> {
    Some(match state {
        RuntimeState::Needed => message(
            "",
            format!("Powermove needs to install {runtime} first."),
            Some(&format!("Install {runtime}")),
        ),
        RuntimeState::Downloading => message("", format!("Downloading {runtime}…"), Some("Cancel")),
        RuntimeState::Activating => message("", format!("Installing {runtime}…"), None),
        RuntimeState::ChecksumMismatch => message(
            "",
            "The download didn’t match its published checksum, so it wasn’t installed.",
            Some("Try Again"),
        ),
        RuntimeState::Offline => message(
            "",
            format!("Connect to the internet to install {runtime}."),
            Some("Try Again"),
        ),
        RuntimeState::RegistryProblem => message(
            "",
            "The update service returned something unexpected. Try again later.",
            Some("Try Again"),
        ),
        RuntimeState::BackgroundFailure => return None,
    })
}

pub fn store_error(code: crate::ServiceErrorCode) -> &'static str {
    match code {
        crate::ServiceErrorCode::Offline => "Can’t reach the Store. Check your connection.",
        _ => "Something went wrong. Try again.",
    }
}
#[derive(Debug, Clone, Copy)]
pub enum StoreFailure {
    Offline,
    RateLimited,
    OldClient,
    Other,
}
pub fn store_failure(failure: StoreFailure) -> &'static str {
    match failure {
        StoreFailure::Offline => "Can’t reach the Store. Check your connection.",
        StoreFailure::RateLimited => "Too many requests. Wait a moment and try again.",
        StoreFailure::OldClient => {
            "This version of Powermove is too old for the Store. Update Powermove and try again."
        }
        StoreFailure::Other => "Something went wrong. Try again.",
    }
}
pub fn empty_search(query: &str) -> Message {
    message(
        format!("No Results for “{query}”"),
        "Try a shorter name, a maker’s handle, or a kind such as “transition”.",
        None,
    )
}
pub fn publish_progress(phase: &Phase) -> Option<String> {
    match phase {
        Phase::Preparing => Some("Getting it ready…".into()),
        Phase::AwaitingConfirmation => Some("Waiting for you to confirm…".into()),
        Phase::Checking => Some("Checking what the Store already has…".into()),
        Phase::Uploading { done, total } => Some(format!("Uploading {done} of {total} files…")),
        Phase::Publishing => Some("Publishing…".into()),
        _ => None,
    }
}
pub fn publish_problem(problem: Problem, last: Option<&str>) -> Option<String> {
    Some(match problem {
        Problem::VersionRequired => "Enter a version.".into(),
        Problem::VersionInvalid => "Use three numbers, like 1.2.0.".into(),
        Problem::VersionNotHigher => format!("Use a version higher than {}.", last?),
        Problem::NotesTooLong => "Keep this under 4,000 characters.".into(),
        Problem::NameRequired => "Enter a name.".into(),
        Problem::NameTooLong => "Keep the name under 80 characters.".into(),
        Problem::TaglineTooLong => "Keep the tagline under 160 characters.".into(),
        Problem::WaiverTooShort => "Say why this is safe to publish.".into(),
        Problem::WaiverTooLong => "Keep the reason under 200 characters.".into(),
        // The owning group displays the finding/check state. No invented copy.
        _ => return None,
    })
}
pub fn sandbox(state: &SandboxState) -> Message {
    match state {
        SandboxState::Running => message(
            "Checking Compatibility",
            "Running it in the sandbox with the permissions it declares.",
            None,
        ),
        SandboxState::CouldNotRun => message(
            "Couldn’t Check",
            "Powermove couldn’t run the sandbox check in this window. Close this and try again.",
            Some("Check Again"),
        ),
        SandboxState::Cancelled => message(
            "Couldn’t Check",
            "The check was cancelled.",
            Some("Check Again"),
        ),
        SandboxState::FullAccess => message(
            "Full Access",
            "Needs full access: installs only after the user trusts it.",
            None,
        ),
        SandboxState::Passed => message(
            "Compatible with the Sandbox",
            "It runs with only the permissions it declares.",
            None,
        ),
        SandboxState::Failed => message(
            "Not Compatible with the Sandbox",
            "The agent can fix this for you, or fix it yourself and check again.",
            Some("Fix with Agent"),
        ),
    }
}
pub fn generation(state: &JobState, cues: usize) -> Option<String> {
    match state {
        JobState::Running { .. } => Some("Generating Captions".into()),
        JobState::Cancelled => Some("Caption generation was cancelled.".into()),
        JobState::Stale => Some("The project changed before captions were ready.".into()),
        JobState::Complete => Some(if cues == 0 {
            "No speech was found in the selected clips.".into()
        } else {
            format!("Added {cues} captions")
        }),
        JobState::Failed { .. } => Some("Couldn’t generate captions.".into()),
        _ => None,
    }
}
pub fn captions_import(count: usize, skipped: usize) -> String {
    let plural = if count == 1 { "caption" } else { "captions" };
    let mut text = format!("Imported {count} {plural}");
    if skipped > 0 {
        text.push_str(&format!(" · {skipped} unreadable skipped"));
    }
    text
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub message: String,
    pub informative: String,
    pub buttons: Vec<String>,
    pub default: usize,
    pub destructive: Option<usize>,
}
pub fn speech_delete(model: &str, size: &str) -> Confirmation {
    Confirmation {
        message: format!("Delete {model}?"),
        informative: format!("This frees {size}. You can download it again at any time."),
        buttons: vec!["Cancel".into(), "Delete".into()],
        default: 0,
        destructive: Some(1),
    }
}
pub fn delete_account() -> Confirmation {
    Confirmation {
        message: "Delete your Powermove account?".into(),
        informative:
            "Your published extensions stay on the Store under your handle. This can’t be undone."
                .into(),
        buttons: vec!["Cancel".into(), "Delete Account".into()],
        default: 0,
        destructive: Some(1),
    }
}
pub fn registry_switch(origin: &str, to_default: bool) -> Confirmation {
    if to_default {
        Confirmation {
            message: "Use Powermove Cloud again?".into(),
            informative:
                "You’ll be signed out, and the Store will show extensions from Powermove Cloud."
                    .into(),
            buttons: vec!["Use Powermove Cloud".into(), "Cancel".into()],
            default: 0,
            destructive: None,
        }
    } else {
        Confirmation{message:format!("Use the registry at {origin}?"),informative:"You’ll be signed out. The Store will show extensions from this registry, and signing in sends your account details to it. Only use a registry you trust.".into(),buttons:vec!["Cancel".into(),"Use This Registry".into()],default:0,destructive:None}
    }
}

pub fn claude_consent(kind: super::client_consent::PromptKind) -> Confirmation {
    match kind{
        super::client_consent::PromptKind::Add=>Confirmation{message:"Add Powermove to the Claude app?".into(),informative:"Claude could then use Powermove’s tools on this Mac, including working in your open projects. You can change this anytime in Settings ▸ Accounts.".into(),buttons:vec!["Don’t Add".into(),"Add to Claude".into()],default:0,destructive:None},
        super::client_consent::PromptKind::SwitchOriginal=>Confirmation{message:"Use Powermove Native with the Claude app?".into(),informative:"Claude is set up to use the original Powermove app. Switching changes that one entry in Claude’s settings, and you can change it anytime in Settings ▸ Accounts.".into(),buttons:vec!["Keep Original".into(),"Switch to Powermove Native".into()],default:0,destructive:None},
    }
}
pub fn external_client(client: super::agent_clients::Client) -> (&'static str, &'static str) {
    use super::agent_clients::Client;
    match client {
        Client::ClaudeCode => (
            "Claude Code",
            "Your Claude plan, in the terminal or your editor.",
        ),
        Client::Codex => ("Codex", "Your ChatGPT plan, in the Codex app or CLI."),
        Client::ClaudeDesktop => ("Claude", "Your Claude plan, in the Claude app."),
        Client::GeminiCli => ("Gemini CLI", "Your Google account or Gemini plan."),
        Client::Cursor => ("Cursor", "Your Cursor plan."),
        Client::Vscode => ("VS Code", "GitHub Copilot’s agent mode."),
        Client::Windsurf => ("Windsurf", "Your Windsurf plan."),
        Client::LmStudio => ("LM Studio", "Models on this Mac, in LM Studio’s chat."),
        Client::Opencode => ("OpenCode", "Any provider you use in OpenCode."),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalClientState {
    NotAdded,
    Adding,
    Removing,
    Added,
    ClaudeOpen,
    Original,
    Handwritten,
    Unreadable,
    Failed { removing: bool },
}
pub fn external_client_row(
    client: super::agent_clients::Client,
    state: ExternalClientState,
) -> Message {
    let (name, detail) = external_client(client);
    match state {
        ExternalClientState::NotAdded => message(name, detail, Some("Add Powermove")),
        ExternalClientState::Adding => message(name, detail, Some("Adding…")),
        ExternalClientState::Removing => message(name, detail, Some("Removing…")),
        ExternalClientState::Added => message(name, detail, Some("Remove")),
        ExternalClientState::ClaudeOpen => {
            message(name, "Quit and reopen Claude to finish.", Some("Remove"))
        }
        ExternalClientState::Original => message(
            name,
            "Claude is using the original Powermove app.",
            Some("Add Powermove"),
        ),
        ExternalClientState::Handwritten => message(name, "Powermove is set up in this app.", None),
        ExternalClientState::Unreadable => message(
            name,
            "Its settings can’t be edited automatically. Use Copy Setup below.",
            None,
        ),
        ExternalClientState::Failed { removing } => message(
            name,
            if client == super::agent_clients::Client::ClaudeDesktop {
                "Couldn’t update Claude’s settings. Check that Claude is installed, then try again."
            } else {
                "Couldn’t change this app’s settings. Try again."
            },
            Some(if removing { "Remove" } else { "Add Powermove" }),
        ),
    }
}
pub fn provider_context(warning: &super::providers::ContextWarning) -> String {
    let digits = warning.tokens.to_string();
    let mut count = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            count.push(',');
        }
        count.push(c);
    }
    format!(
        "Ollama is running {} with only {count} tokens of context, so it will lose track of Powermove’s instructions. In Ollama’s settings, set Context length to 32k or more (or set OLLAMA_CONTEXT_LENGTH=32768), then test again.",
        warning.model
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_steps_and_errors_use_opus_copy() {
        assert_eq!(
            sign_in(SignInStep::Handle, false, "Google", "")
                .unwrap()
                .title,
            "Choose Your Handle"
        );
        assert_eq!(
            auth_error(AuthError::ExpiredCode, "", ""),
            "That code has expired. Send a new one."
        );
        assert_eq!(
            auth_error(AuthError::HandleTaken, "", "abc"),
            "@abc is taken. Try another."
        );
        assert_eq!(resend(2), "Resend in 2 s");
        assert_eq!(resend(0), "Resend Code");
        assert!(sign_in(SignInStep::Closed, false, "", "").is_none());
    }
    #[test]
    fn runtime_and_connection_states_do_not_invent_background_banners() {
        assert_eq!(
            connection(ProviderState::ApiUnset, "API / Local").title,
            "Bring Your Own Model"
        );
        assert_eq!(
            connection(ProviderState::Reconnecting, "Codex").line,
            "Connection interrupted. Reconnecting…"
        );
        assert_eq!(
            runtime(RuntimeState::ChecksumMismatch, "Codex")
                .unwrap()
                .line,
            "The download didn’t match its published checksum, so it wasn’t installed."
        );
        assert!(runtime(RuntimeState::BackgroundFailure, "Codex").is_none());
    }
    #[test]
    fn store_and_publish_copy_keeps_casing_and_measured_counts() {
        assert_eq!(
            store_failure(StoreFailure::Offline),
            "Can’t reach the Store. Check your connection."
        );
        assert_eq!(
            publish_progress(&Phase::Uploading { done: 1, total: 2 }).unwrap(),
            "Uploading 1 of 2 files…"
        );
        assert_eq!(
            sandbox(&SandboxState::Cancelled).line,
            "The check was cancelled."
        );
        assert_eq!(
            publish_problem(Problem::NotesTooLong, None).unwrap(),
            "Keep this under 4,000 characters."
        );
    }
    #[test]
    fn caption_outcome_copy_distinguishes_cancel_stale_and_no_speech() {
        assert_eq!(
            generation(&JobState::Stale, 0).unwrap(),
            "The project changed before captions were ready."
        );
        assert_eq!(
            generation(&JobState::Complete, 0).unwrap(),
            "No speech was found in the selected clips."
        );
        assert_eq!(
            captions_import(1, 2),
            "Imported 1 caption · 2 unreadable skipped"
        );
    }
    #[test]
    fn destructive_and_registry_confirmation_defaults_match_opus() {
        let delete = delete_account();
        assert_eq!(delete.buttons[delete.default], "Cancel");
        assert_ne!(Some(delete.default), delete.destructive);
        let speech = speech_delete("Model", "1 GB");
        assert_eq!(speech.message, "Delete Model?");
        assert_eq!(
            speech.informative,
            "This frees 1 GB. You can download it again at any time."
        );
        assert_eq!(speech.buttons[speech.default], "Cancel");
        let custom = registry_switch("https://example.com", false);
        assert_eq!(custom.buttons[custom.default], "Cancel");
        assert_eq!(custom.message, "Use the registry at https://example.com?");
        let back = registry_switch("", true);
        assert_eq!(back.buttons[back.default], "Use Powermove Cloud");
    }
    #[test]
    fn new_s070_consent_defaults_and_failed_remove_copy_match_spec() {
        let prompt = claude_consent(super::super::client_consent::PromptKind::Add);
        assert_eq!(prompt.message, "Add Powermove to the Claude app?");
        assert_eq!(prompt.buttons[prompt.default], "Don’t Add");
        let prompt = claude_consent(super::super::client_consent::PromptKind::SwitchOriginal);
        assert_eq!(prompt.buttons[prompt.default], "Keep Original");
        let row = external_client_row(
            super::super::agent_clients::Client::ClaudeDesktop,
            ExternalClientState::Failed { removing: true },
        );
        assert_eq!(row.action.as_deref(), Some("Remove"));
        assert_eq!(
            row.line,
            "Couldn’t update Claude’s settings. Check that Claude is installed, then try again."
        );
        let warning = super::super::providers::ContextWarning {
            model: "m".into(),
            tokens: 4096,
            minimum: 16384,
            recommended: 32768,
        };
        assert!(provider_context(&warning).contains("only 4,096 tokens of context"));
    }
}
