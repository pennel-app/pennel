#ifndef PENNEL_TYPING_FFI_H
#define PENNEL_TYPING_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Inputs are borrowed, readable UTF-8 buffers of the explicit length, at most 1 MiB.
 * Responses are nonnull Rust-owned UTF-8 JSON C strings. Free exactly once using
 * pennel_typing_free_response, including responses that describe invalid input.
 * A null pointer or excessive length returns an error without reading the input. */
char *pennel_typing_request_json(const uint8_t *bytes, size_t length);
char *pennel_typing_swipe_json(const uint8_t *bytes, size_t length);
void pennel_typing_free_response(char *response);

#ifdef __cplusplus
}
#endif

#endif
