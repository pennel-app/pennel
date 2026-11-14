cmake_minimum_required(VERSION 3.28)
project(pennel_decider LANGUAGES C CXX ASM)
if(NOT APPLE OR NOT CMAKE_OSX_ARCHITECTURES STREQUAL "arm64")
  message(FATAL_ERROR "This runtime profile supports Apple Silicon macOS only")
endif()
if(NOT PENNEL_LLAMA_SOURCE_DIR OR NOT PENNEL_DECIDER_BUILD_SIGNATURE)
  message(FATAL_ERROR "Pinned source and build signature are required")
endif()
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
add_subdirectory("${PENNEL_LLAMA_SOURCE_DIR}" llama EXCLUDE_FROM_ALL)
add_library(pennel_decider SHARED owned_decider.cpp)
target_link_libraries(pennel_decider PRIVATE llama)
target_compile_definitions(pennel_decider PRIVATE PENNEL_DECIDER_BUILD_SIGNATURE="${PENNEL_DECIDER_BUILD_SIGNATURE}")
target_compile_options(pennel_decider PRIVATE -Wall -Wextra -Werror)
target_link_options(pennel_decider PRIVATE "LINKER:-exported_symbols_list,${CMAKE_CURRENT_SOURCE_DIR}/exports.txt")
set_target_properties(pennel_decider PROPERTIES
  OUTPUT_NAME pennel_decider
  INSTALL_NAME_DIR "@rpath"
  MACOSX_RPATH ON
  BUILD_WITH_INSTALL_NAME_DIR ON
)
