# Aria 0.4.1: the update check without GitHub's API

Approved by the user on 2026-11-26 ("Yes, fix it"), after the short design in the conversation.

## Why

0.3.5's first real check, against v0.4.0, failed with "GitHub's answer didn't make sense". The
cause was GitHub's API: it allows 60 requests an hour per IP address without an account, and
something else on the user's network had used all 60. GitHub answered `403 rate limit exceeded`,
and Aria shows every refusal as an answer that makes no sense. Aria's own log has only two checks
that day. Behind a shared address (a carrier's NAT, an office, a university), that limit is used
up often, so the check fails for reasons that have nothing to do with Aria.

## Goals

- The check no longer uses `api.github.com`. Two ordinary github.com links replace it, and
  neither has the API's 60-an-hour limit:
  - `https://github.com/Hixenov/Aria/releases/latest`, read without following the redirect. The
    `302` `Location` is `https://github.com/Hixenov/Aria/releases/tag/<tag>`. The tag gives the
    version, and the location is the release page.
  - The release's files, under `https://github.com/Hixenov/Aria/releases/download/<tag>/`:
    `SHA256SUMS.txt`, then `Aria-<version>-setup.exe`.
  On 2026-11-26 both worked while the API refused this address (checked with curl).
- A refusal says what it is. `403` and `429` become a new error, `Busy`, worded "GitHub is busy
  right now, so Aria will try again later". Like `Offline`, it is retried in an hour while
  automatic checks are on. A click stays one check.
- Everything else stays as it is: when Aria checks, the one-time question, the SHA-256 check
  before keeping and again before starting the installer, "Restart to update", tell-only for
  portable, loose, macOS and Linux, and the plain `Aria` User-Agent.

## Non-goals and anti-goals

- No GitHub account or token, and no other server.
- No retry loop sooner than the hour already used.
- Nothing about the tray (it comes next, separately).

## The how (my calls; the user approved the direction)

- `Endpoints` becomes `{ latest, tag_prefix, download_prefix }`, with `tag_prefix` set to
  `https://github.com/Hixenov/Aria/releases/tag/`.
  - The redirect's location must start with `tag_prefix`, and the rest is the tag.
  - A tag that is not a plain version is `NotARelease`, as before.
  - No redirect, or a location elsewhere, is `Answer`.
- Without the API, a file's size isn't known before it downloads.
  - `Asset` loses `size`.
  - The installer's download takes its total from `Content-Length`.
    - It refuses a total over the cap before reading.
    - It never reads past the cap.
    - When there is a `Content-Length`, it keeps only exactly that many bytes.
  - The checksum stays the real check.
- "Still being published" was "the API lists no `SHA256SUMS.txt` yet". Now it is:
  - `SHA256SUMS.txt` answering `404` (a new error, `Missing`, which the service turns into
    `NotReady`), or
  - a checksum list that does not name the installer.
- `parse_release` (JSON) and its fixture go. `serde_json` leaves `aria-update` if nothing else
  there uses it.
- The Updates entry in PRIVACY.md names github.com instead of its API.

## Builder's leeway

- Names of private helpers.
- How the fake server in the tests serves a redirect.
- The wording in Russian and Armenian.
