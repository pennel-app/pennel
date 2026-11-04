import React, { useEffect, useRef } from 'react';
import { ActivityIndicator, Alert, KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Switch, Text, TextInput, View, type TextInputProps, type TextProps, type ViewStyle } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import Ionicons from '@expo/vector-icons/Ionicons';
import { Host, Menu, Picker, Button as NativeButton, Text as NativeText } from '@expo/ui/swift-ui';
import { accessibilityLabel, buttonStyle, disabled as nativeDisabled, font, frame, pickerStyle, tag } from '@expo/ui/swift-ui/modifiers';
import { useRuntime } from '../runtime';
import { ownedNativeFailureText } from '../errors';
import { useRouter } from 'expo-router';
import { clock } from '../format';
import { recordingPresentation } from '../workflow-state';
import { workflowText } from '../workflow-copy';

export function Body({ children, muted = false, style, ...props }: TextProps & { muted?: boolean }) {
  const { theme } = useRuntime();
  return <Text {...props} style={[{ color: muted ? theme.colors.muted : theme.colors.foreground, fontFamily: theme.font.regular, fontSize: theme.size.section, lineHeight: theme.space[6] }, style]}>{children}</Text>;
}

export function Page({ title, subtitle, children, action, footer, scroll = true, inset = true, bottomInset = false, scrollKey, recordingStatus = true }: { title?: string; subtitle?: string; children: React.ReactNode; action?: React.ReactNode; footer?: React.ReactNode; scroll?: boolean; inset?: boolean; bottomInset?: boolean; scrollKey?: string; recordingStatus?: boolean }) {
  const { theme, error, store, t, language } = useRuntime();
  const scrollView = useRef<React.ComponentRef<typeof ScrollView>>(null);
  const offsets = useRef(new Map<string, number>());
  useEffect(() => {
    if (!scroll || !scrollKey) return;
    const frame = requestAnimationFrame(() => scrollView.current?.scrollTo({ y: offsets.current.get(scrollKey) ?? 0, animated: false }));
    return () => cancelAnimationFrame(frame);
  }, [scroll, scrollKey]);
  const content = <>
    {(title || subtitle || action) && <View style={{ paddingBottom: theme.space[5], flexDirection: 'row', alignItems: 'center', gap: theme.space[3] }}><View style={{ flex: 1 }}>{title && <Text accessibilityRole="header" style={{ fontFamily: theme.font.display, fontSize: theme.size['display-title'], color: theme.colors.foreground }}>{title}</Text>}{subtitle && <Body muted style={{ marginTop: title ? theme.space[2] : 0 }}>{subtitle}</Body>}</View>{action}</View>}
    {error && <View accessibilityRole="alert" style={{ padding: theme.space[3], marginBottom: theme.space[4], backgroundColor: theme.colors.subtle, borderRadius: theme.radius.control }}><Body style={{ color: theme.colors.accent }}>{ownedNativeFailureText(language, error)}</Body><Button title={t('common.close')} subtle onPress={store.clearError} /></View>}
    {children}
  </>;
  return <SafeAreaView edges={[...(inset ? ['top' as const] : []), 'left', 'right', ...(bottomInset ? ['bottom' as const] : [])]} style={{ flex: 1, backgroundColor: theme.colors.surface }}><KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} style={{ flex: 1 }}>{recordingStatus && <RecordingStatus />}{scroll ? <ScrollView ref={scrollView} keyboardShouldPersistTaps="handled" onScroll={scrollKey ? (event) => { offsets.current.set(scrollKey, event.nativeEvent.contentOffset.y); } : undefined} scrollEventThrottle={32} contentContainerStyle={{ padding: theme.space[5], paddingBottom: theme.space[10] }} automaticallyAdjustKeyboardInsets>{content}</ScrollView> : <View style={{ flex: 1, padding: theme.space[5], paddingBottom: bottomInset ? theme.space[3] : 0 }}>{content}</View>}{footer && <View style={{ paddingHorizontal: theme.space[5], paddingVertical: theme.space[3], borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: theme.colors.border, backgroundColor: theme.colors.surface }}>{footer}</View>}</KeyboardAvoidingView></SafeAreaView>;
}

function RecordingStatus() {
  const { snapshot, theme, t, language } = useRuntime();
  const router = useRouter();
  const meeting = snapshot?.meeting;
  if (!meeting || !meeting.id && !meeting.retained_id || meeting.phase === 'idle' && !meeting.retained_id) return null;
  const retained = Boolean(meeting.retained_id);
  return <Pressable accessibilityRole="button" accessibilityLabel={`${t('mobile.simple.recording_return')}. ${meeting.title}`} accessibilityHint={t('mobile.simple.recording_return_help')} onPress={() => router.push('/recording')} style={{ minHeight: 52, flexDirection: 'row', alignItems: 'center', gap: theme.space[3], paddingVertical: theme.space[3], marginHorizontal: theme.space[5], borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: theme.colors.border }}>
    <Ionicons accessible={false} name={retained ? 'alert-circle-outline' : 'radio-button-on'} size={20} color={theme.colors.accent} />
    <View style={{ flex: 1 }}><Body style={{ fontFamily: theme.font.medium }}>{t('mobile.simple.recording_return')}</Body><Body muted style={{ fontSize: theme.size.content }}>{retained ? t('mobile.simple.recording_retained') : `${workflowText(language, recordingPresentation(meeting.phase).status)} · ${clock(meeting.elapsed_secs)}`}</Body></View>
    <Ionicons accessible={false} name="chevron-forward" size={18} color={theme.colors.muted} />
  </Pressable>;
}

export function Section({ title, description, children }: { title?: string; description?: string; children?: React.ReactNode }) {
  const { theme } = useRuntime();
  return <View style={{ marginBottom: theme.space[7], gap: theme.space[3] }}>{title && <Text accessibilityRole="header" style={{ fontFamily: theme.font.semibold, fontSize: theme.size.section, color: theme.colors.foreground }}>{title}</Text>}{children}{description && <Body muted style={{ fontSize: theme.size.content, lineHeight: theme.space[5] }}>{description}</Body>}</View>;
}

export function Button({ title, onPress, disabled = false, busy = false, subtle = false, destructive = false, icon }: { title: string; onPress: () => void; disabled?: boolean; busy?: boolean; subtle?: boolean; destructive?: boolean; icon?: React.ComponentProps<typeof Ionicons>['name'] }) {
  const { theme } = useRuntime();
  const inactive = disabled || busy;
  return <Pressable accessible accessibilityRole="button" accessibilityLabel={title} accessibilityState={{ disabled: inactive, busy }} disabled={inactive} onPress={onPress} style={({ pressed }) => ({ minHeight: 44, paddingHorizontal: theme.space[4], paddingVertical: theme.space[3], borderRadius: theme.radius.control, backgroundColor: inactive ? theme.colors.disabled : subtle ? pressed ? theme.colors.hover : 'transparent' : theme.colors.button, opacity: pressed && !subtle ? 0.8 : 1, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: theme.space[2] })}>{busy && <ActivityIndicator color={subtle ? theme.colors.muted : theme.colors['button-text']} />}{icon && <Ionicons accessible={false} importantForAccessibility="no-hide-descendants" name={icon} size={20} color={inactive ? theme.colors['disabled-text'] : subtle ? destructive ? theme.colors.accent : theme.colors.foreground : theme.colors['button-text']} />}<Text style={{ fontFamily: theme.font.medium, fontSize: theme.size.section, color: inactive ? theme.colors['disabled-text'] : subtle ? destructive ? theme.colors.accent : theme.colors.foreground : theme.colors['button-text'], textAlign: 'center' }}>{title}</Text></Pressable>;
}

export function Field({ label, ...props }: TextInputProps & { label: string }) {
  const { theme } = useRuntime();
  return <View style={{ gap: theme.space[2] }}><Text style={{ fontFamily: theme.font.medium, fontSize: theme.size.content, color: theme.colors.muted }}>{label}</Text><TextInput accessibilityLabel={label} placeholderTextColor={theme.colors.muted} selectionColor={theme.colors.accent} {...props} style={[{ minHeight: 44, padding: theme.space[3], borderRadius: theme.radius.control, backgroundColor: theme.colors.field, fontFamily: theme.font.regular, fontSize: theme.size.section, color: theme.colors.foreground, textAlignVertical: props.multiline ? 'top' : 'center' }, props.style]} /></View>;
}

export function Row({ title, subtitle, onPress, trailing, disabled = false }: { title: string; subtitle?: string; onPress?: () => void; trailing?: React.ReactNode; disabled?: boolean }) {
  const { theme } = useRuntime();
  const label = <View style={{ flex: 1 }}><Body style={{ fontFamily: theme.font.medium, color: disabled ? theme.colors['disabled-text'] : theme.colors.foreground }}>{title}</Body>{subtitle && <Body muted style={{ fontSize: theme.size.content }}>{subtitle}</Body>}</View>;
  return <View style={{ minHeight: 52, paddingVertical: onPress ? 0 : theme.space[3], flexDirection: 'row', gap: theme.space[3], alignItems: 'center', borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: theme.colors.border }}>
    {onPress ? <Pressable accessible accessibilityRole="button" accessibilityLabel={subtitle ? `${title}. ${subtitle}` : title} accessibilityState={{ disabled }} disabled={disabled} onPress={onPress} style={{ flex: 1, minHeight: 52, paddingVertical: theme.space[3], flexDirection: 'row', gap: theme.space[3], alignItems: 'center' }}>
      {label}
      <Ionicons accessible={false} importantForAccessibility="no-hide-descendants" name="chevron-forward" size={18} color={theme.colors.muted} />
    </Pressable> : label}
    {trailing}
  </View>;
}

export function Toggle({ title, value, onChange, description, disabled = false }: { title: string; value: boolean; onChange: (value: boolean) => void; description?: string; disabled?: boolean }) {
  const { theme } = useRuntime();
  return <Row title={title} subtitle={description} disabled={disabled} trailing={<Switch accessibilityLabel={title} value={value} disabled={disabled} onValueChange={onChange} trackColor={{ false: theme.colors.selection, true: theme.colors.accent }} />} />;
}

export function Choice<T extends string | number>({ label, value, options, onChange }: { label: string; value: T; options: readonly { value: T; label: string; disabled?: boolean }[]; onChange: (value: T) => void }) {
  const { theme } = useRuntime();
  return <View style={{ gap: theme.space[2] }}><Body muted style={{ fontSize: theme.size.content }}>{label}</Body><View accessibilityRole="radiogroup" style={{ gap: theme.space[1] }}>{options.map((option) => <Pressable key={option.value} accessible accessibilityRole="radio" accessibilityLabel={option.label} accessibilityState={{ checked: value === option.value, disabled: option.disabled }} disabled={option.disabled} onPress={() => onChange(option.value)} style={{ minHeight: 44, padding: theme.space[3], borderRadius: theme.radius.control, backgroundColor: value === option.value ? theme.colors.selection : 'transparent', flexDirection: 'row', alignItems: 'center', gap: theme.space[3] }}><Ionicons accessible={false} importantForAccessibility="no-hide-descendants" name={value === option.value ? 'radio-button-on' : 'radio-button-off'} size={20} color={option.disabled ? theme.colors['disabled-text'] : theme.colors.foreground} /><Body style={{ flex: 1, color: option.disabled ? theme.colors['disabled-text'] : theme.colors.foreground }}>{option.label}</Body></Pressable>)}</View></View>;
}

export function NativeChoice<T extends string | number>({ label, value, options, onChange, segmented = false }: { label: string; value: T; options: readonly { value: T; label: string; disabled?: boolean }[]; onChange: (value: T) => void; segmented?: boolean }) {
  const { theme } = useRuntime();
  if (Platform.OS !== 'ios') return <Choice label={label} value={value} options={options} onChange={onChange} />;
  return <Host colorScheme={theme.mode} seedColor={theme.colors.foreground} matchContents={{ vertical: true }} style={{ minHeight: 44 }}>
    <Picker label={label} selection={value} onSelectionChange={(next) => { if (options.some((option) => option.value === next && !option.disabled)) onChange(next); }} modifiers={[pickerStyle(segmented ? 'segmented' : 'menu'), frame({ minHeight: 44, maxWidth: Infinity }), font({ family: theme.font.medium, textStyle: 'body', size: theme.size.content })]}>
      {options.map((option) => <NativeText key={option.value} modifiers={[tag(option.value), nativeDisabled(Boolean(option.disabled))]}>{option.label}</NativeText>)}
    </Picker>
  </Host>;
}

export interface MenuAction { label: string; onPress: () => void; disabled?: boolean; destructive?: boolean }
export function ActionMenu({ label, actions }: { label: string; actions: readonly MenuAction[] }) {
  const { theme, t } = useRuntime();
  if (Platform.OS !== 'ios') return <Button title={label} subtle icon="ellipsis-horizontal" onPress={() => Alert.alert(label, undefined, [...actions.map((action) => ({ text: action.label, style: action.destructive ? 'destructive' as const : 'default' as const, onPress: action.disabled ? undefined : action.onPress })), { text: t('common.cancel'), style: 'cancel' }])} />;
  return <Host colorScheme={theme.mode} seedColor={theme.colors.foreground} matchContents style={{ minHeight: 44, minWidth: 44 }}><Menu label={label} systemImage="ellipsis.circle" modifiers={[buttonStyle('plain'), frame({ minHeight: 44 }), font({ family: theme.font.medium, textStyle: 'body', size: theme.size.content }), accessibilityLabel(label)]}>{actions.map((action) => <NativeButton key={action.label} label={action.label} role={action.destructive ? 'destructive' : 'default'} onPress={action.onPress} modifiers={[nativeDisabled(Boolean(action.disabled))]} />)}</Menu></Host>;
}

export function Notice({ children, error = false }: { children: React.ReactNode; error?: boolean }) { const { theme } = useRuntime(); return <Body accessibilityRole={error ? 'alert' : undefined} muted={!error} style={{ fontSize: theme.size.content, color: error ? theme.colors.accent : theme.colors.muted }}>{children}</Body>; }
export function Loading() { const { theme } = useRuntime(); return <ActivityIndicator style={{ margin: theme.space[6] }} color={theme.colors.foreground} />; }
export function Empty({ title, description }: { title: string; description?: string }) { const { theme } = useRuntime(); return <View style={{ paddingVertical: theme.space[8], gap: theme.space[2] }}><Body>{title}</Body>{description && <Body muted>{description}</Body>}</View>; }
export function Actions({ children }: { children: React.ReactNode }) { const { theme } = useRuntime(); return <View style={{ flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: theme.space[2] }}>{children}</View>; }
export function confirm(title: string, message: string, action: string, cancel: string, onConfirm: () => void, destructive = false): void { Alert.alert(title, message, [{ text: cancel, style: 'cancel' }, { text: action, style: destructive ? 'destructive' : 'default', onPress: onConfirm }]); }
