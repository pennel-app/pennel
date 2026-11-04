import React from 'react';
import { Redirect, useLocalSearchParams } from 'expo-router';

export default function LegacyAsk() {
  const params = useLocalSearchParams<{ conversation_id?: string; message_id?: string }>();
  return <Redirect href={{ pathname: '/(tabs)', params: { pane: 'questions', conversation_id: params.conversation_id, message_id: params.message_id } }} />;
}
