function targetBundleIdentifiers(bundleIdentifier) {
  return {
    Pennel: bundleIdentifier,
    ExpoWidgetsTarget: `${bundleIdentifier}.widgets`,
    PennelKeyboard: `${bundleIdentifier}.keyboard`,
    PennelKeyboardTests: `${bundleIdentifier}.keyboard-tests`,
    PennelRuntimeTests: `${bundleIdentifier}.runtime-tests`,
  };
}

function resolveMobileVariant(environment = process.env) {
  const production = environment.PENNEL_VARIANT === 'production';
  const testFlight = environment.PENNEL_TESTFLIGHT === '1';
  if (testFlight && production) throw new Error('TestFlight distribution is restricted to Pennel Preview');
  return {
    production, testFlight,
    bundleIdentifier: production ? 'app.pennel.ios' : 'app.pennel.ios.preview',
    groupIdentifier: production ? 'group.app.pennel' : 'group.app.pennel.preview',
    scheme: production ? 'pennel' : 'pennel-preview',
  };
}

module.exports = { resolveMobileVariant, targetBundleIdentifiers };
