#!/usr/bin/env ruby
# scripts/fix_test_targets.rb
#
# Ensures all .swift files in the 'Tests' directory are ONLY members of the
# 'jharaTests' target and NOT the main 'jhara' app target.
#
# Requirement: gem install xcodeproj

require 'xcodeproj'
require 'fileutils'

PROJECT_PATH = File.expand_path('../apps/macos/jhara.xcodeproj', __dir__)
TEST_DIR_PART = 'Tests/' # Path fragment for test files

# 1. Open project
unless File.exist?(PROJECT_PATH)
  puts "Error: Could not find Xcode project at #{PROJECT_PATH}"
  exit 1
end

project = Xcodeproj::Project.open(PROJECT_PATH)

# 2. Identify Targets
main_target = project.targets.find { |t| t.name == 'jhara' }
test_target = project.targets.find { |t| t.name == 'jharaTests' }

unless main_target && test_target
  puts "Error: Could not find both 'jhara' and 'jharaTests' targets."
  puts "Available targets: #{project.targets.map(&:name).join(', ')}"
  exit 1
end

puts "Found targets: #{main_target.name}, #{test_target.name}"

# 3. Process Files
changed = false

project.files.each do |file_ref|
  # Try both real_path and simple path for robust matching
  path = file_ref.path.to_s
  real_path = file_ref.real_path.to_s rescue ""
  
  is_test_file = (path.include?(TEST_DIR_PART) || real_path.include?(TEST_DIR_PART)) && 
                 (path.end_with?('.swift') || real_path.end_with?('.swift'))

  if is_test_file
    filename = File.basename(path.empty? ? real_path : path)
    puts "Checking test file: #{filename} (Path: #{path})"
    
    # Ensure it's in the test target's 'Compile Sources'
    unless test_target.source_build_phase.files_references.include?(file_ref)
      puts "  -> Adding to #{test_target.name}..."
      test_target.add_file_references([file_ref])
      changed = true
    end
    
    # Ensure it's NOT in the main target's 'Compile Sources'
    main_target.source_build_phase.files.each do |build_file|
      if build_file.file_ref == file_ref
        puts "  -> Removing from #{main_target.name} (it belongs in tests)..."
        main_target.source_build_phase.remove_build_file(build_file)
        changed = true
      end
    end
  end
end

# 4. Save
if changed
  project.save
  puts "Success: Project targets updated."
else
  puts "No changes needed. Test files are already correctly mapped."
end
