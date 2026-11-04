import React, { useEffect, useState } from 'react';
import { Modal } from 'react-native';
import { useRuntime } from '../runtime';
import { Actions, Button, Field, Notice, Page } from './ui';

export function TextDialog({ visible, title, label, initial = '', help, multiline = false, onClose, onSave }: { visible: boolean; title: string; label: string; initial?: string; help?: string; multiline?: boolean; onClose: () => void; onSave: (value: string) => Promise<boolean> }) {
  const { t } = useRuntime();
  const [value, setValue] = useState(initial);
  const [busy, setBusy] = useState(false);
  useEffect(() => { if (visible) setValue(initial); }, [initial, visible]);
  return <Modal visible={visible} presentationStyle="pageSheet" onRequestClose={onClose} animationType="slide"><Page recordingStatus={false} title={title}><Field label={label} value={value} onChangeText={setValue} autoFocus multiline={multiline} autoCapitalize="sentences" />{help && <Notice>{help}</Notice>}<Actions><Button title={t('common.cancel')} subtle disabled={busy} onPress={onClose} /><Button title={t('common.save')} busy={busy} onPress={() => { setBusy(true); void onSave(value).then((saved) => { if (saved) onClose(); }).finally(() => setBusy(false)); }} /></Actions></Page></Modal>;
}
