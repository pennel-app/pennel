import type { ExpoConfig, ConfigContext } from 'expo/config';
import tokens from '../../assets/brand/tokens.json';
import { resolveMobileVariant, targetBundleIdentifiers } from './plugins/mobile-variant.cjs';

export default ({ config }: ConfigContext): ExpoConfig => {
  const { production, testFlight, bundleIdentifier, groupIdentifier, scheme } = resolveMobileVariant();
  return {
    ...config,
    name: 'Pennel',
    slug: 'pennel-mobile',
    version: '0.1.0',
    orientation: 'default',
    scheme,
    userInterfaceStyle: 'automatic',
    icon: './artifacts/brand/app-icon.png',
    backgroundColor: tokens.colors.surface.light,
    locales: { en: './locales/en.json', nl: './locales/nl.json' },
    ios: {
      bundleIdentifier,
      appleTeamId: testFlight ? 'B538Q2K76C' : undefined,
      supportsTablet: true,
      buildNumber: '1',
      entitlements: { 'com.apple.security.application-groups': [groupIdentifier] },
      infoPlist: {
        // The owner-confirmed internal Preview excludes France; see docs/mobile-testflight.md.
        ...(testFlight ? { ITSAppUsesNonExemptEncryption: false } : {}),
        CFBundleDisplayName: production ? 'Pennel' : 'Pennel Preview',
        NSMicrophoneUsageDescription: 'Pennel uses your microphone for dictation and meeting recordings you start.',
        NSCameraUsageDescription: 'Pennel uses your camera to scan a temporary device connection code. You can enter the code instead.',
        NSSpeechRecognitionUsageDescription: 'Pennel uses Apple speech recognition when you select the Apple provider.',
        UIBackgroundModes: ['audio'],
        BGTaskSchedulerPermittedIdentifiers: [`${bundleIdentifier}.sync`, `${bundleIdentifier}.finish`],
        NSSupportsLiveActivities: true,
        UIFileSharingEnabled: true,
        LSSupportsOpeningDocumentsInPlace: true,
        PennelAppGroupIdentifier: groupIdentifier,
        PennelHandoffNamespace: production ? 'app.pennel.handoff' : `${bundleIdentifier}.handoff`,
        PennelKeychainService: bundleIdentifier,
        PennelURLScheme: scheme,
        PennelSyncTaskIdentifier: `${bundleIdentifier}.sync`,
        PennelFinishTaskIdentifier: `${bundleIdentifier}.finish`,
      },
    },
    android: { package: production ? 'app.pennel.android' : 'app.pennel.android.preview' },
    plugins: [
      'expo-router',
      ['expo-font', { fonts: [
        '../../assets/brand/fonts/Figtree-Regular.ttf',
        '../../assets/brand/fonts/Figtree-Medium.ttf',
        '../../assets/brand/fonts/Figtree-SemiBold.ttf',
        '../../assets/brand/fonts/InstrumentSerif-Regular.ttf',
      ] }],
      ['expo-widgets', {
        bundleIdentifier: targetBundleIdentifiers(bundleIdentifier).ExpoWidgetsTarget,
        groupIdentifier,
        enablePushNotifications: false,
        widgets: [],
      }],
      ['./plugins/with-pennel.cjs', { bundleIdentifier, groupIdentifier, scheme, production, testFlight }],
    ],
    experiments: { typedRoutes: true },
    extra: { pennelVariant: production ? 'production' : 'preview' },
  };
};
