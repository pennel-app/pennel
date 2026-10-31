const fs = require('node:fs');
const path = require('node:path');

const serviceFiles = [
  'AppModel.swift', 'AppPaths.swift', 'AppRouter.swift', 'Keychain.swift', 'ErrorText.swift', 'ProviderKinds.swift',
  'Audio/SharedMicrophone.swift', 'Audio/MicCapture.swift',
  'Dictation/DictationRunner.swift', 'Dictation/FirstRunDefaults.swift',
  'Session/DictationSession.swift', 'Session/HandoffServer.swift', 'Session/SessionActivity.swift',
  'Meetings/MeetingRecorder.swift', 'Meetings/MeetingRuntime.swift', 'Meetings/MeetingShare.swift', 'Meetings/RecordingActivity.swift',
  'Cloud/CloudAccountModel.swift', 'Cloud/CloudFormat.swift', 'Cloud/CloudSyncRunner.swift', 'Cloud/KeychainSecretStore.swift',
  'System/AppleIntelligence.swift', 'System/AppleSpeech.swift', 'System/AppleSpeechPreview.swift', 'Onboarding/OnboardingFlow.swift',
  'Keyboard/KeyboardWritingServer.swift',
  'Settings/SessionDurationSetting.swift', 'Intents/IntentEnvironment.swift',
];

function link(source, destination, directory = false) {
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.symlinkSync(path.relative(path.dirname(destination), source), destination, directory ? 'dir' : 'file');
}

function prepareNativeSources(workspace) {
  const generated = path.join(workspace, 'modules/pennel-mobile/generated/ios');
  // CocoaPods ignores source patterns escaping the pod root. These links keep one canonical edited source.
  fs.rmSync(generated, { recursive: true, force: true });
  for (const file of serviceFiles) {
    const source = path.join(workspace, 'native/ios/Services', file);
    if (!fs.existsSync(source)) throw new Error(`Required native service source is missing: ${file}`);
    link(source, path.join(generated, 'Services', file));
  }
  link(path.join(workspace, 'artifacts/ios/PennelCore/pennel_core.swift'), path.join(generated, 'Bindings/pennel_core.swift'));
  link(path.join(workspace, 'artifacts/ios/PennelCoreFFI.xcframework'), path.join(generated, 'PennelCoreFFI.xcframework'), true);
  const typing = path.join(workspace, 'native/ios/Typing/Generated/PennelTypingFFI.xcframework');
  fs.mkdirSync(path.dirname(typing), { recursive: true });
  const stagedTyping = fs.lstatSync(typing, { throwIfNoEntry: false });
  if (stagedTyping) {
    if (!stagedTyping.isSymbolicLink()) throw new Error('Pennel typing artifact staging must remain a generated symbolic link');
    fs.unlinkSync(typing);
  }
  link(path.join(workspace, 'artifacts/ios/PennelTypingFFI.xcframework'), typing, true);
  return generated;
}

if (require.main === module) {
  prepareNativeSources(path.resolve(__dirname, '..'));
  process.stdout.write(`Prepared pod-visible links for ${serviceFiles.length} native service sources and the matching core artifacts.\n`);
}

module.exports = { prepareNativeSources, serviceFiles };
