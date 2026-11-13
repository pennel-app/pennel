import React from 'react';
import { Platform, Text, View } from 'react-native';
import Ionicons from '@expo/vector-icons/Ionicons';
import { markdownBlocks, type MarkdownSpan } from '../markdown';
import { useRuntime } from '../runtime';
import { Body } from './ui';

const monospace = Platform.select({ ios: 'Menlo', default: 'monospace' });

function Spans({ spans }: { spans: MarkdownSpan[] }) {
  const { theme } = useRuntime();
  return <>{spans.map((span, index) => span.kind === 'text' ? span.text
    : <Text key={index} style={span.kind === 'bold' ? { fontFamily: theme.font.semibold } : { fontFamily: monospace, fontSize: theme.size.content, backgroundColor: theme.colors.subtle }}>{span.text}</Text>)}</>;
}

export function Markdown({ text, muted = false }: { text: string; muted?: boolean }) {
  const { theme, t } = useRuntime();
  const marker = { width: theme.space[5], alignItems: 'flex-start' as const, paddingTop: theme.space[1] };
  return <View style={{ gap: theme.space[1] }}>{markdownBlocks(text).map((block, index) => {
    if (block.kind === 'gap') return <View key={index} style={{ height: theme.space[2] }} />;
    if (block.kind === 'heading') return <Body key={index} selectable muted={muted} accessibilityRole="header"
      style={{ fontFamily: theme.font.semibold, fontSize: block.level === 2 ? theme.size.title : theme.size.section, lineHeight: block.level === 2 ? theme.space[7] : theme.space[6], marginTop: index ? theme.space[3] : 0 }}><Spans spans={block.spans} /></Body>;
    if (block.kind === 'paragraph') return <Body key={index} selectable muted={muted}><Spans spans={block.spans} /></Body>;
    return <View key={index} style={{ flexDirection: 'row' }}>
      {block.kind === 'task'
        ? <View style={marker}><Ionicons name={block.done ? 'checkbox-outline' : 'square-outline'} size={16} color={muted ? theme.colors.muted : theme.colors.foreground} accessible accessibilityRole="image" accessibilityLabel={t(block.done ? 'dom.task_done' : 'dom.task_open')} /></View>
        : <Body muted={muted} accessible={false} style={{ width: theme.space[5] }}>•</Body>}
      <Body selectable muted={muted} style={{ flex: 1 }}><Spans spans={block.spans} /></Body>
    </View>;
  })}</View>;
}
