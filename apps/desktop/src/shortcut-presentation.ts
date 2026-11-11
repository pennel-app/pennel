import type { AppInfo, ShortcutRow } from './api';
import { domBindingAvailable, keyboardLayout } from './shortcut-layout.ts';
import { t, type TranslationKey, localizeBackendText } from './i18n.ts';
export function shortcutRow(info: AppInfo, action: string): ShortcutRow | undefined {
  const row = info.shortcuts?.find(row => row.id === action);
  if (row?.scope === 'application' && info.platform !== 'macos' && row.binding && row.capability.available && !domBindingAvailable(row.binding, keyboardLayout())) {
    return { ...row, capability: { available: false, reason: 'dom_layout_unavailable' } };
  }
  return row;
}
export function shortcutText(info: AppInfo, action: string): string {
  const row = shortcutRow(info, action);
  return row?.binding ? row.label.text : t('shortcuts.none');
}
export function shortcutKeycaps(info: AppInfo, action: string): string[] { return shortcutRow(info, action)?.binding ? shortcutRow(info, action)!.label.keycaps : []; }
export function shortcutName(action: string): string {
  const key = `shortcuts.action.${action}` as TranslationKey;
  return t(key);
}
export function shortcutReason(reason: string | null | undefined): string {
  if (!reason) return '';
  const reasons: Record<string, TranslationKey> = {
    dom_layout_unavailable: 'shortcuts.reason.layout', next_model_not_ready: 'shortcuts.reason.second_model', second_model_required: 'shortcuts.reason.second_model', shortcut_input_active: 'shortcuts.reason.input_active', native_shortcut_unavailable: 'shortcuts.reason.native_capture', shortcut_registration_failed: 'shortcuts.reason.registration', shortcut_release_failed: 'shortcuts.reason.registration',
    wayland_toggle_dictation: 'shortcuts.reason.wayland_dictation', wayland_toggle_meeting: 'shortcuts.reason.wayland_record', wayland_open_pennel: 'shortcuts.reason.wayland_open', wayland_global_shortcuts: 'shortcuts.reason.wayland',
    x11_dictate_modifier_only: 'shortcuts.reason.x11_dictate', x11_keyed_dictation: 'shortcuts.reason.x11_dictate', native_key_not_supported: 'shortcuts.reason.native_key',
    accessibility_required: 'shortcuts.reason.accessibility', native_capture_unavailable: 'shortcuts.reason.native_capture',
    unknown_action: 'shortcuts.reason.invalid', unknown_modifier: 'shortcuts.reason.invalid', key_and_code_required: 'shortcuts.reason.invalid', unknown_key: 'shortcuts.reason.native_key', sides_require_modifier_only: 'shortcuts.reason.sides', unavailable: 'shortcuts.reason.native_capture', shift_is_command: 'shortcuts.reason.shift', system_shortcut: 'shortcuts.reason.reserved',
    primary_modifier_required: 'shortcuts.reason.primary', key_required: 'shortcuts.reason.key', shift_alone: 'shortcuts.reason.shift',
    modifier_required: 'shortcuts.reason.modifier', fn_modifier_only_macos: 'shortcuts.reason.fn', menu_key_not_supported: 'shortcuts.reason.native_key',
  };
  return reasons[reason] ? t(reasons[reason]) : localizeBackendText(reason);
}
/** Shift is inserted beside the core's canonical modifier keycaps; the base label remains core-owned. */
export function commandKeycaps(info: AppInfo): string[] {
  const dictate = shortcutRow(info, 'dictate');
  if (!dictate?.binding) return [];
  const keys = [...dictate.label.keycaps];
  const beforeShift = dictate.binding.mods.filter(mod => ['ctrl','alt'].includes(mod.replace(/^(left|right)_/, ''))).length;
  keys.splice(beforeShift, 0, info.platform === 'macos' ? '⇧' : 'Shift');
  return keys;
}
