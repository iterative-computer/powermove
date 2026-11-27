#pragma once
#include <mach/mach.h>
#include <stdint.h>
// A Mach port right, never an address or global IOSurface ID, crosses the
// process boundary. Both ends validate this exact message before accepting it.
static const mach_msg_id_t STUDIO_SURFACE_MESSAGE = 0x504d5301;
struct StudioSurfaceMessage {
    mach_msg_header_t header;
    mach_msg_body_t body;
    mach_msg_port_descriptor_t surface;
    uint64_t frame;
    uint32_t width;
    uint32_t height;
};
