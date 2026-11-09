// The only Objective-C dependency of Studio's optional temporal upscaler.
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <MetalFX/MetalFX.h>
#import <simd/simd.h>
#import <IOSurface/IOSurface.h>
#include <servers/bootstrap.h>
#include "surface-message.h"
#include <cstring>
#include <cstdio>
#include <cstdlib>
#include <initializer_list>
#include <array>
struct SurfaceSlot {IOSurfaceRef surface=nullptr;id<MTLTexture> texture;uint64_t frame=0;};
struct StudioMetalFX {
    id<MTLFXTemporalDenoisedScaler> scaler;
    id<MTLCommandQueue> queue;
    mach_port_t surfaceReceiver=MACH_PORT_NULL;
    std::array<SurfaceSlot,3> surfaces;
};
extern "C" void *studio_metalfx_create(void *rawDevice, unsigned iw, unsigned ih, unsigned ow, unsigned oh) {
    @autoreleasepool {
        if (@available(macOS 26.0, *)) {
            id<MTLDevice> device=(__bridge id<MTLDevice>)rawDevice;
            if (![MTLFXTemporalDenoisedScalerDescriptor supportsDevice:device]) return nullptr;
            MTLFXTemporalDenoisedScalerDescriptor *d=[MTLFXTemporalDenoisedScalerDescriptor new];
            d.inputWidth=iw;d.inputHeight=ih;d.outputWidth=ow;d.outputHeight=oh;
            d.colorTextureFormat=MTLPixelFormatRGBA16Float;d.depthTextureFormat=MTLPixelFormatR32Float;
            d.motionTextureFormat=MTLPixelFormatRG32Float;d.diffuseAlbedoTextureFormat=MTLPixelFormatRGBA16Float;
            d.specularAlbedoTextureFormat=MTLPixelFormatRGBA16Float;d.normalTextureFormat=MTLPixelFormatRGBA16Float;
            d.roughnessTextureFormat=MTLPixelFormatR32Float;d.outputTextureFormat=MTLPixelFormatRGBA16Float;
            d.autoExposureEnabled=YES;d.requiresSynchronousInitialization=YES;
            id<MTLFXTemporalDenoisedScaler> scaler=[d newTemporalDenoisedScalerWithDevice:device];
            if (!scaler) return nullptr;
            auto *ctx=new StudioMetalFX;ctx->scaler=scaler;ctx->queue=[device newCommandQueue];
            const char*service=getenv("STUDIO_SURFACE_SERVICE");if(service)bootstrap_look_up(bootstrap_port,service,&ctx->surfaceReceiver);
            return ctx;
        }
        return nullptr;
    }
}
extern "C" bool studio_metalfx_encode(void *context,void **textures,const float *view,const float *projection,float jitterX,float jitterY,bool reset) {
    @autoreleasepool {
        if (@available(macOS 26.0, *)) {
            auto *ctx=static_cast<StudioMetalFX*>(context);auto s=ctx->scaler;
            s.colorTexture=(__bridge id<MTLTexture>)textures[0];s.depthTexture=(__bridge id<MTLTexture>)textures[1];
            s.motionTexture=(__bridge id<MTLTexture>)textures[2];s.diffuseAlbedoTexture=(__bridge id<MTLTexture>)textures[3];
            s.specularAlbedoTexture=(__bridge id<MTLTexture>)textures[4];s.normalTexture=(__bridge id<MTLTexture>)textures[5];
            s.roughnessTexture=(__bridge id<MTLTexture>)textures[6];s.outputTexture=(__bridge id<MTLTexture>)textures[7];
            s.preExposure=1.f;s.motionVectorScaleX=1.f;s.motionVectorScaleY=1.f;
            s.jitterOffsetX=jitterX;s.jitterOffsetY=jitterY;s.shouldResetHistory=reset;s.depthReversed=NO;
            simd_float4x4 v,p;std::memcpy(&v,view,64);std::memcpy(&p,projection,64);s.worldToViewMatrix=v;s.viewToClipMatrix=p;
            id<MTLCommandBuffer> command=[ctx->queue commandBuffer];[s encodeToCommandBuffer:command];[command commit];[command waitUntilCompleted];
            return command.status==MTLCommandBufferStatusCompleted;
        }
        return false;
    }
}
extern "C" bool studio_metalfx_send(void *context,void *rawTexture,uint64_t frame) {
    @autoreleasepool {
        auto*ctx=static_cast<StudioMetalFX*>(context);if(ctx->surfaceReceiver==MACH_PORT_NULL)return false;
        id<MTLTexture> source=(__bridge id<MTLTexture>)rawTexture;
        SurfaceSlot*slot=nullptr;for(auto&candidate:ctx->surfaces){if(candidate.frame==0){slot=&candidate;break;}}if(!slot)return false;
        if(!slot->surface){
            NSDictionary *properties=@{(id)kIOSurfaceWidth:@(source.width),(id)kIOSurfaceHeight:@(source.height),(id)kIOSurfaceBytesPerElement:@4,(id)kIOSurfacePixelFormat:@((uint32_t)'RGBA')};
            slot->surface=IOSurfaceCreate((__bridge CFDictionaryRef)properties);if(!slot->surface)return false;
            MTLTextureDescriptor *d=[MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm width:source.width height:source.height mipmapped:NO];d.usage=MTLTextureUsageShaderRead|MTLTextureUsageShaderWrite;d.storageMode=MTLStorageModeShared;
            slot->texture=[source.device newTextureWithDescriptor:d iosurface:slot->surface plane:0];
            if(!slot->texture){CFRelease(slot->surface);slot->surface=nullptr;return false;}
        }
        id<MTLCommandBuffer> command=[ctx->queue commandBuffer];id<MTLBlitCommandEncoder> blit=[command blitCommandEncoder];
        [blit copyFromTexture:source sourceSlice:0 sourceLevel:0 sourceOrigin:MTLOriginMake(0,0,0) sourceSize:MTLSizeMake(source.width,source.height,1) toTexture:slot->texture destinationSlice:0 destinationLevel:0 destinationOrigin:MTLOriginMake(0,0,0)];[blit endEncoding];[command commit];[command waitUntilCompleted];
        if(command.status!=MTLCommandBufferStatusCompleted)return false;
        mach_port_t port=IOSurfaceCreateMachPort(slot->surface);if(port==MACH_PORT_NULL)return false;
        StudioSurfaceMessage message={};message.header.msgh_bits=MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND,0)|MACH_MSGH_BITS_COMPLEX;message.header.msgh_size=sizeof(message);message.header.msgh_remote_port=ctx->surfaceReceiver;message.header.msgh_id=STUDIO_SURFACE_MESSAGE;message.body.msgh_descriptor_count=1;message.surface.name=port;message.surface.disposition=MACH_MSG_TYPE_MOVE_SEND;message.surface.type=MACH_MSG_PORT_DESCRIPTOR;message.frame=frame;message.width=source.width;message.height=source.height;
        kern_return_t result=mach_msg(&message.header,MACH_SEND_MSG|MACH_SEND_TIMEOUT,sizeof(message),0,MACH_PORT_NULL,0,MACH_PORT_NULL);if(result!=KERN_SUCCESS){mach_port_deallocate(mach_task_self(),port);return false;}slot->frame=frame;return true;
    }
}
extern "C" void studio_metalfx_release(void*context,uint64_t frame){auto*ctx=static_cast<StudioMetalFX*>(context);for(auto&slot:ctx->surfaces){if(slot.frame==frame)slot.frame=0;}}
extern "C" void studio_metalfx_destroy(void *context) {auto*ctx=static_cast<StudioMetalFX*>(context);if(ctx->surfaceReceiver!=MACH_PORT_NULL)mach_port_deallocate(mach_task_self(),ctx->surfaceReceiver);for(auto&slot:ctx->surfaces)if(slot.surface)CFRelease(slot.surface);delete ctx;}
extern "C" uint64_t studio_metal_allocated_bytes(void *rawDevice){id<MTLDevice>device=(__bridge id<MTLDevice>)rawDevice;return device.currentAllocatedSize;}
