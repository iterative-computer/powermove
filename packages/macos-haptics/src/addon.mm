#import <AppKit/AppKit.h>
#import <CoreText/CoreText.h>
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

static napi_value FontFamilies(napi_env env, napi_callback_info info) {
  CFArrayRef families = CTFontManagerCopyAvailableFontFamilyNames();
  napi_value result;
  CFIndex count = families ? CFArrayGetCount(families) : 0;
  napi_create_array_with_length(env, count, &result);
  for (CFIndex i = 0; i < count; i++) {
    NSString *family = (NSString *)CFArrayGetValueAtIndex(families, i);
    napi_value value;
    napi_create_string_utf8(env, [family UTF8String], NAPI_AUTO_LENGTH, &value);
    napi_set_element(env, result, i, value);
  }
  if (families) CFRelease(families);
  return result;
}

static napi_value Init(napi_env env, napi_value exports) {
  napi_value trigger;
  napi_create_function(env, "triggerAlignment", NAPI_AUTO_LENGTH,
                       TriggerAlignment, nullptr, &trigger);
  napi_set_named_property(env, exports, "triggerAlignment", trigger);
  napi_value fonts;
  napi_create_function(env, "fontFamilies", NAPI_AUTO_LENGTH, FontFamilies, nullptr, &fonts);
  napi_set_named_property(env, exports, "fontFamilies", fonts);
  return exports;
}

NAPI_MODULE(NODE_GYP_MODULE_NAME, Init)
