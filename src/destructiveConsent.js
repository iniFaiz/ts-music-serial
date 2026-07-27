import { invokeCommand as invoke } from './generated/ipc';

// Native OS confirmation is the security boundary for high-impact operations.
// The returned token is short-lived and bound by Rust to the action/targets.
export async function requestDestructiveConsent(action, targets = []) {
  return invoke('request_destructive_consent', {
    action,
    targets: Array.isArray(targets) ? targets : [],
  });
}
