#import <AppKit/AppKit.h>
#include <node_api.h>

static void PerformAlignmentFeedback(void) {
  [[NSHapticFeedbackManager defaultPerformer]
      performFeedbackPattern:NSHapticFeedbackPatternAlignment
             performanceTime:NSHapticFeedbackPerformanceTimeNow];
}

static napi_value TriggerAlignment(napi_env env, napi_callback_info info) {
  if ([NSThread isMainThread]) {
    PerformAlignmentFeedback();
  } else {
    dispatch_async(dispatch_get_main_queue(), ^{
      PerformAlignmentFeedback();
    });
  }

  napi_value undefined;
  napi_get_undefined(env, &undefined);
  return undefined;
}

static napi_value Init(napi_env env, napi_value exports) {
  napi_value trigger;
  napi_create_function(env, "triggerAlignment", NAPI_AUTO_LENGTH,
                       TriggerAlignment, nullptr, &trigger);
  napi_set_named_property(env, exports, "triggerAlignment", trigger);
  return exports;
}

NAPI_MODULE(NODE_GYP_MODULE_NAME, Init)
