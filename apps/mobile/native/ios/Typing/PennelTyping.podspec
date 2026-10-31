Pod::Spec.new do |s|
  s.name = 'PennelTyping'
  s.version = '0.1.0'
  s.summary = 'Bounded local Pennel typing policy'
  s.homepage = 'https://github.com/pennel-app/pennel'
  s.license = { :type => 'Apache-2.0' }
  s.author = 'Pennel'
  s.source = { :git => 'https://github.com/pennel-app/pennel.git', :tag => s.version.to_s }
  s.platform = :ios, '26.0'
  s.swift_version = '6.0'
  s.static_framework = true
  s.source_files = 'Sources/PennelTyping/**/*.swift'
  s.vendored_frameworks = 'Generated/PennelTypingFFI.xcframework'
  s.frameworks = 'Foundation'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES', 'SWIFT_COMPILATION_MODE' => 'wholemodule' }
end
