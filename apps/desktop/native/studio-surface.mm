// Stable N-API only: this addon needs no Electron/V8 ABI rebuild. It receives
// IOSurface Mach rights in main and returns a process-local IOSurfaceRef Buffer.
#include <node_api.h>
#import <Foundation/Foundation.h>
#import <IOSurface/IOSurface.h>
#include <servers/bootstrap.h>
#include <mach/mach.h>
#include <bsm/libbsm.h>
#include <cmath>
#include "surface-message.h"
#include <string>
#include <cstdio>

static napi_value fail(napi_env env,const char*message){napi_throw_error(env,nullptr,message);return nullptr;}
static napi_value number(napi_env env,uint32_t n){napi_value v;napi_create_uint32(env,n,&v);return v;}
static napi_value str(napi_env env,const char*s){napi_value v;napi_create_string_utf8(env,s,NAPI_AUTO_LENGTH,&v);return v;}
static void prop(napi_env env,napi_value o,const char*k,napi_value v){napi_set_named_property(env,o,k,v);}
static napi_value create(napi_env env,napi_callback_info info){
    mach_port_t port=MACH_PORT_NULL;
    if(mach_port_allocate(mach_task_self(),MACH_PORT_RIGHT_RECEIVE,&port)!=KERN_SUCCESS)return fail(env,"Could not allocate Studio surface receiver");
    if(mach_port_insert_right(mach_task_self(),port,port,MACH_MSG_TYPE_MAKE_SEND)!=KERN_SUCCESS){mach_port_destroy(mach_task_self(),port);return fail(env,"Could not create Studio surface send right");}
    char service[128];snprintf(service,sizeof(service),"com.zellzoi.powermove.studio.%u.%s",getpid(),NSUUID.UUID.UUIDString.UTF8String);
    kern_return_t result=bootstrap_register(bootstrap_port,service,port);
    if(result!=KERN_SUCCESS){mach_port_destroy(mach_task_self(),port);return fail(env,"Could not register Studio surface receiver");}
    napi_value value;napi_create_object(env,&value);prop(env,value,"port",number(env,port));prop(env,value,"service",str(env,service));return value;
}
struct SurfaceHandle {IOSurfaceRef surface;};
// Ownership belongs to this Buffer object, not its pooled backing-store
// address. Views can alias that address and GC can recycle it independently.
static const napi_type_tag surfaceTag={0xe6d1475da8904f13ULL,0xb7cf5b9d6028ac44ULL};
static void finalize(napi_env env,void*data,void*hint){auto*h=static_cast<SurfaceHandle*>(data);if(h->surface)CFRelease(h->surface);delete h;}
static napi_value take(napi_env env,napi_callback_info info){
    size_t argc=3;napi_value args[3];napi_get_cb_info(env,info,&argc,args,nullptr,nullptr);uint32_t port=0,expectedPid=0;double wanted=0.;
    if(argc!=3||napi_get_value_uint32(env,args[0],&port)!=napi_ok||napi_get_value_double(env,args[1],&wanted)!=napi_ok||napi_get_value_uint32(env,args[2],&expectedPid)!=napi_ok||!expectedPid||!std::isfinite(wanted)||wanted<0.||wanted>9007199254740991.||std::floor(wanted)!=wanted)return fail(env,"Invalid Studio surface request");
    for(int attempt=0;attempt<8;attempt++){
        struct Receive {StudioSurfaceMessage message;mach_msg_max_trailer_t trailer;} received={};
        kern_return_t result=mach_msg(&received.message.header,MACH_RCV_MSG|MACH_RCV_TIMEOUT|MACH_RCV_TRAILER_TYPE(MACH_MSG_TRAILER_FORMAT_0)|MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT),0,sizeof(received),port,0,MACH_PORT_NULL);
        if(result==MACH_RCV_TIMED_OUT){napi_value v;napi_get_null(env,&v);return v;}
        if(result!=KERN_SUCCESS)return fail(env,"Studio surface receive failed");
        auto&m=received.message;
        if(m.header.msgh_id!=STUDIO_SURFACE_MESSAGE||m.header.msgh_size!=sizeof(StudioSurfaceMessage)||!(m.header.msgh_bits&MACH_MSGH_BITS_COMPLEX)||m.body.msgh_descriptor_count!=1||m.surface.type!=MACH_MSG_PORT_DESCRIPTOR){mach_msg_destroy(&m.header);continue;}
        // The bootstrap name is only a routing hint. Authenticate the sender
        // with the kernel-provided audit trailer, never message body fields.
        auto*trailer=reinterpret_cast<mach_msg_audit_trailer_t*>(reinterpret_cast<uint8_t*>(&m)+round_msg(m.header.msgh_size));
        if(trailer->msgh_trailer_type!=MACH_MSG_TRAILER_FORMAT_0||trailer->msgh_trailer_size<sizeof(mach_msg_audit_trailer_t)||(uint32_t)audit_token_to_pid(trailer->msgh_audit)!=expectedPid){mach_msg_destroy(&m.header);continue;}
        IOSurfaceRef surface=IOSurfaceLookupFromMachPort(m.surface.name);mach_port_deallocate(mach_task_self(),m.surface.name);
        if(!surface)continue;
        if(m.frame!=(uint64_t)wanted||IOSurfaceGetWidth(surface)!=m.width||IOSurfaceGetHeight(surface)!=m.height){CFRelease(surface);continue;}
        auto*handle=new SurfaceHandle{surface};napi_value buffer;void*bytes=nullptr;
        if(napi_create_buffer_copy(env,sizeof(IOSurfaceRef),&surface,&bytes,&buffer)!=napi_ok||napi_type_tag_object(env,buffer,&surfaceTag)!=napi_ok||napi_wrap(env,buffer,handle,finalize,nullptr,nullptr)!=napi_ok){CFRelease(surface);delete handle;return fail(env,"Could not retain Studio surface");}
        napi_value value;napi_create_object(env,&value);prop(env,value,"handle",buffer);prop(env,value,"width",number(env,m.width));prop(env,value,"height",number(env,m.height));return value;
    }
    return fail(env,"Studio surface frame is missing");
}
static napi_value release(napi_env env,napi_callback_info info){
    size_t argc=1;napi_value arg;napi_get_cb_info(env,info,&argc,&arg,nullptr,nullptr);void*data=nullptr;size_t size=0;
    if(argc!=1||napi_get_buffer_info(env,arg,&data,&size)!=napi_ok||size!=sizeof(IOSurfaceRef))return fail(env,"Invalid Studio surface handle");
    bool owned=false;SurfaceHandle*h=nullptr;
    if(napi_check_object_type_tag(env,arg,&surfaceTag,&owned)!=napi_ok||!owned||napi_unwrap(env,arg,reinterpret_cast<void**>(&h))!=napi_ok||!h)return fail(env,"Unknown Studio surface handle");
    if(h->surface){CFRelease(h->surface);h->surface=nullptr;*static_cast<IOSurfaceRef*>(data)=nullptr;}
    napi_value v;napi_get_undefined(env,&v);return v;
}
static napi_value closeReceiver(napi_env env,napi_callback_info info){
    size_t argc=2;napi_value args[2];napi_get_cb_info(env,info,&argc,args,nullptr,nullptr);uint32_t port=0;char name[128];size_t length=0;
    if(argc!=2||napi_get_value_uint32(env,args[0],&port)!=napi_ok||napi_get_value_string_utf8(env,args[1],name,sizeof(name),&length)!=napi_ok)return fail(env,"Invalid Studio receiver");
    bootstrap_register(bootstrap_port,name,MACH_PORT_NULL);mach_port_destroy(mach_task_self(),port);napi_value v;napi_get_undefined(env,&v);return v;
}
static napi_value init(napi_env env,napi_value exports){
    napi_property_descriptor methods[]={{"create",0,create,0,0,0,napi_default,0},{"take",0,take,0,0,0,napi_default,0},{"release",0,release,0,0,0,napi_default,0},{"close",0,closeReceiver,0,0,0,napi_default,0}};
    napi_define_properties(env,exports,4,methods);return exports;
}
NAPI_MODULE(NODE_GYP_MODULE_NAME,init)
