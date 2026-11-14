require 'json'

package = JSON.parse(File.read(File.join(__dir__, 'package.json')))
peer_qa = ENV.fetch('PENNEL_PEER_TRANSPORT_QA', '0')
raise 'PENNEL_PEER_TRANSPORT_QA must be 0 or 1' unless ['0', '1'].include?(peer_qa)
if peer_qa == '1' && (ENV['PENNEL_VARIANT'] == 'production' || ENV['PENNEL_TESTFLIGHT'] == '1')
  raise 'Peer transport qualification is restricted to local Pennel Preview builds'
end
manifest_path = File.join(__dir__, '../../artifacts/ios/manifest.json')
if File.exist?(manifest_path)
  manifest = JSON.parse(File.read(manifest_path))
  raise 'Core preparation is incomplete; prepare matching complete artifacts before installing Pods' if manifest['preparing'] == true
  features = manifest.fetch('features')
  unless features.include?('peer-transport-qa') == (peer_qa == '1')
    raise 'Prepare matching ordinary or peer-transport-qa core artifacts before installing Pods'
  end
elsif peer_qa == '1'
  raise 'Prepare peer-transport-qa core artifacts before installing Pods'
end

Pod::Spec.new do |s|
  s.name = 'PennelMobile'
  s.version = package['version']
  s.summary = 'Pennel native runtime and shared Rust core for Expo'
  s.homepage = 'https://github.com/pennel-app/pennel'
  s.license = { :type => 'Apache-2.0' }
  s.author = 'Pennel'
  s.source = { :git => 'https://github.com/pennel-app/pennel.git', :tag => s.version.to_s }
  s.platform = :ios, '26.0'
  s.swift_version = '6.0'
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.dependency 'PennelMobileKit'
  s.source_files = [
    'ios/**/*.swift',
    'generated/ios/Bindings/*.swift',
    'generated/ios/Services/**/*.swift'
  ]
  s.vendored_frameworks = 'generated/ios/PennelCoreFFI.xcframework'
  s.frameworks = 'AVFoundation', 'Speech', 'FoundationModels', 'ActivityKit', 'BackgroundTasks', 'AppIntents', 'Security', 'SystemConfiguration'
  s.libraries = 'c++', 'resolv'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES', 'SWIFT_COMPILATION_MODE' => 'wholemodule' }
end
