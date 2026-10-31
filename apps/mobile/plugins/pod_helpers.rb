require 'cocoapods-core'

module PennelPortablePodspecChecksums
  def checksum
    return super unless root? && ['ExpoModulesCore', 'ExpoWidgets'].include?(name) &&
                        defined_in_file && File.basename(defined_in_file.to_s) == "#{name}.podspec.json"

    @checksum ||= begin
      require 'digest'
      # Expo saves absolute checkout paths in these evaluated specs; keep their build attributes intact.
      workspace = Regexp.escape(File.expand_path('..', __dir__))
      location = %r{(?:(?<=file://)|(?<=["'=\s])|\A)#{workspace}(?=/|"|\\["nt]|$)}
      input = File.read(defined_in_file).gsub(location, '/__pennel_mobile_workspace__')
      Digest::SHA1.hexdigest(input).encode('UTF-8')
    end
  end
end

unless Pod::Specification.ancestors.include?(PennelPortablePodspecChecksums)
  Pod::Specification.prepend(PennelPortablePodspecChecksums)
end

def pennel_post_install(installer)
  native_targets = installer.pods_project.targets.group_by(&:name)
  installer.pod_targets.each do |pod|
    targets = native_targets[pod.label] || []
    next if targets.empty?
    next if ['PennelMobile', 'PennelMobileKit'].include?(pod.pod_name)
    declared_versions = pod.specs.flat_map do |spec|
      Array(spec.attributes_hash['swift_versions'] || spec.attributes_hash['swift_version'])
    end.compact
    next unless declared_versions.empty?

    # Unspecified SDK pods otherwise inherit Swift 6 from Pennel instead of Expo's stock Swift 5 consumer.
    targets.each do |target|
      target.build_configurations.each do |configuration|
        configuration.build_settings['SWIFT_VERSION'] = '5.0'
      end
    end
  end
end
