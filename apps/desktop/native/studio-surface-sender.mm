// Test-only peer: sends a small surface from a process other than the app.
#import <Foundation/Foundation.h>
#import <IOSurface/IOSurface.h>
#include <servers/bootstrap.h>
#include <mach/mach.h>
#include "surface-message.h"

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    @autoreleasepool {
        mach_port_t receiver = MACH_PORT_NULL;
        if (bootstrap_look_up(bootstrap_port, argv[1], &receiver) != KERN_SUCCESS) return 3;
        IOSurfaceRef surface = IOSurfaceCreate((__bridge CFDictionaryRef)@{
            (id)kIOSurfaceWidth: @4, (id)kIOSurfaceHeight: @4,
            (id)kIOSurfaceBytesPerElement: @4, (id)kIOSurfacePixelFormat: @((uint32_t)'RGBA')
        });
        if (!surface) return 4;
        mach_port_t port = IOSurfaceCreateMachPort(surface);
        StudioSurfaceMessage message = {};
        message.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
        message.header.msgh_size = sizeof(message);
        message.header.msgh_remote_port = receiver;
        message.header.msgh_id = STUDIO_SURFACE_MESSAGE;
        message.body.msgh_descriptor_count = 1;
        message.surface.name = port;
        message.surface.disposition = MACH_MSG_TYPE_MOVE_SEND;
        message.surface.type = MACH_MSG_PORT_DESCRIPTOR;
        message.frame = 42; message.width = 4; message.height = 4;
        kern_return_t result = mach_msg(&message.header, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof(message), 0, MACH_PORT_NULL, 1000, MACH_PORT_NULL);
        if (result != KERN_SUCCESS) mach_port_deallocate(mach_task_self(), port);
        mach_port_deallocate(mach_task_self(), receiver); CFRelease(surface);
        return result == KERN_SUCCESS ? 0 : 5;
    }
}
