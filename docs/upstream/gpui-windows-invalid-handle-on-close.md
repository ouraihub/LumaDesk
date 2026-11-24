# Draft: comment for longbridge/gpui-component#3795

Not posted. Post as a comment on https://github.com/longbridge/gpui-component/issues/3795
(or open a Zed issue against `crates/gpui_windows`, which `gpui-pre-windows` snapshots).

---

Same symptom here on Windows 11 with `gpui-kit 0.9.1` → `gpui-pre-windows 0.3.12`, and the code is
unchanged in `gpui-kit 0.11.0` → `gpui-pre-windows 0.3.14`. Closing the **only** window (not just a
secondary one) logs five ERRORs every time, all `ERROR_INVALID_WINDOW_HANDLE`:

| Source | Call | Error |
|---|---|---|
| `window.rs:701` | `mouse_position` → `ScreenToClient(self.0.hwnd, …).ok().log_err()` | `0x80070578` |
| `window.rs:616` | `Drop` → `ShowWindowAsync(self.0.hwnd, SW_HIDE).ok().log_err()` | `0x80070578` |
| `platform.rs:480` | VSync loop → `RedrawWindow(...).ok().log_err()` (this issue) | `0x80070578` |
| `window.rs:628` | `Drop` task → `RevokeDragDrop(handle).log_err()` | `0x80040102` |
| `window.rs:629` | `Drop` task → `DestroyWindow(handle).log_err()` | `0x80070578` |
| `dispatcher.rs:129` | `PostMessageW(platform_window_handle, WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD, …)` | `0x80070578` |

Repro: any app with one window and `QuitMode::LastWindowClosed`; send `WM_CLOSE` (click ×).

Besides the `IsWindow` guard proposed above for the VSync loop, the same guard (or treating
`ERROR_INVALID_WINDOW_HANDLE` as expected during teardown) would be needed for the `Drop` path,
`mouse_position` and the dispatcher wake-up, since by then the HWND is already gone. None of these
failures affect behavior; they only make every normal close look like a crash in the log.
