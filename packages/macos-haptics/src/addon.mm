#import <AppKit/AppKit.h>
#import <CoreText/CoreText.h>
#include <node_api.h>
#include <sys/stat.h>
#include <string>
#include <vector>

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

// Metadata checks never open the data fork, so they cannot hydrate a placeholder.
struct CloudWork {
  napi_async_work work;
  napi_deferred deferred;
  std::string path, state, error;
  bool download;
};
static void InspectCloud(napi_env env, void *data) {
  CloudWork *job = static_cast<CloudWork *>(data);
  @autoreleasepool {
    NSString *path = [NSString stringWithUTF8String:job->path.c_str()];
    NSURL *url = [NSURL fileURLWithPath:path];
    struct stat st;
    int result = stat(job->path.c_str(), &st);
    int statError = errno;
    NSNumber *ubiquitous = nil;
    [url getResourceValue:&ubiquitous forKey:NSURLIsUbiquitousItemKey error:nil];
    NSString *status = nil;
    if ([ubiquitous boolValue]) {
      [url getResourceValue:&status forKey:NSURLUbiquitousItemDownloadingStatusKey error:nil];
    }
    BOOL offloaded = (result == 0 && (st.st_flags & SF_DATALESS)) ||
      [status isEqualToString:NSURLUbiquitousItemDownloadingStatusNotDownloaded];
    // Older iCloud versions leave a hidden .filename.icloud placeholder.
    if (result != 0 && statError == ENOENT && !offloaded) {
      NSString *stub = [[path stringByDeletingLastPathComponent]
        stringByAppendingPathComponent:[NSString stringWithFormat:@".%@.icloud", [path lastPathComponent]]];
      NSURL *stubURL = [NSURL fileURLWithPath:stub];
      NSNumber *stubCloud = nil;
      [stubURL getResourceValue:&stubCloud forKey:NSURLIsUbiquitousItemKey error:nil];
      if ([stubCloud boolValue]) { offloaded = YES; ubiquitous = @YES; }
    }
    job->state = offloaded ? ([ubiquitous boolValue] ? "icloud" : "cloud") :
      result == 0 ? (S_ISREG(st.st_mode) ? "local" : "unknown") :
      statError == ENOENT ? "missing" : "unknown";
    if (job->download && offloaded && [ubiquitous boolValue]) {
      NSError *error = nil;
      if (![[NSFileManager defaultManager] startDownloadingUbiquitousItemAtURL:url error:&error]) {
        job->error = [[error localizedDescription] UTF8String] ?: "Could not start iCloud download";
      }
    }
  }
}
static void CloudComplete(napi_env env, napi_status status, void *data) {
  CloudWork *job = static_cast<CloudWork *>(data);
  napi_value value;
  if (status != napi_ok || !job->error.empty()) {
    napi_value message;
    napi_create_string_utf8(env, job->error.empty() ? "Cloud operation cancelled" : job->error.c_str(), NAPI_AUTO_LENGTH, &message);
    napi_create_error(env, nullptr, message, &value);
    napi_reject_deferred(env, job->deferred, value);
  } else {
    napi_create_string_utf8(env, job->state.c_str(), NAPI_AUTO_LENGTH, &value);
    napi_resolve_deferred(env, job->deferred, value);
  }
  napi_delete_async_work(env, job->work);
  delete job;
}
static napi_value CloudFileState(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value args[2];
  napi_get_cb_info(env, info, &argc, args, nullptr, nullptr);
  size_t length = 0;
  if (argc < 1 || napi_get_value_string_utf8(env, args[0], nullptr, 0, &length) != napi_ok || length > 16384) {
    napi_throw_type_error(env, nullptr, "Expected a file path"); return nullptr;
  }
  std::vector<char> path(length + 1);
  napi_get_value_string_utf8(env, args[0], path.data(), path.size(), &length);
  CloudWork *job = new CloudWork{};
  job->path = std::string(path.data(), length);
  if (argc > 1) napi_get_value_bool(env, args[1], &job->download);
  napi_value promise, name;
  napi_create_promise(env, &job->deferred, &promise);
  napi_create_string_utf8(env, "cloudFileState", NAPI_AUTO_LENGTH, &name);
  napi_create_async_work(env, nullptr, name, InspectCloud, CloudComplete, job, &job->work);
  napi_queue_async_work(env, job->work);
  return promise;
}

static napi_value Init(napi_env env, napi_value exports) {
  napi_value trigger;
  napi_create_function(env, "triggerAlignment", NAPI_AUTO_LENGTH,
                       TriggerAlignment, nullptr, &trigger);
  napi_set_named_property(env, exports, "triggerAlignment", trigger);
  napi_value fonts;
  napi_create_function(env, "fontFamilies", NAPI_AUTO_LENGTH, FontFamilies, nullptr, &fonts);
  napi_set_named_property(env, exports, "fontFamilies", fonts);
  napi_value cloud;
  napi_create_function(env, "cloudFileState", NAPI_AUTO_LENGTH, CloudFileState, nullptr, &cloud);
  napi_set_named_property(env, exports, "cloudFileState", cloud);
  return exports;
}

NAPI_MODULE(NODE_GYP_MODULE_NAME, Init)
