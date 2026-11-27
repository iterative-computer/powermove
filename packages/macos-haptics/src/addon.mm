#import <AppKit/AppKit.h>
#import <CoreText/CoreText.h>
#include <node_api.h>
#include <sys/stat.h>
#include <sys/clonefile.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdio.h>
#include <string>
#include <vector>
#include <atomic>
#include <mutex>

// Core Text publishes process changes locally and user/session changes through
// the distributed center. Only advance a counter here: notifications may arrive
// off the Node thread, where N-API and JavaScript values cannot be accessed.
static std::atomic<uint32_t> fontRegistryVersion{0};
static std::mutex fontObserverMutex;
static uint32_t fontObserverEnvironments = 0;
struct FontObserverEnvironment { bool watching = false; };
static void FontRegistryChanged(CFNotificationCenterRef center, void *observer,
                                CFStringRef name, const void *object, CFDictionaryRef info) {
  fontRegistryVersion.fetch_add(1, std::memory_order_relaxed);
}
static void RemoveFontObservers(void *data) {
  FontObserverEnvironment *environment = static_cast<FontObserverEnvironment *>(data);
  bool watching = environment->watching;
  delete environment;
  if (!watching) return;
  std::lock_guard<std::mutex> lock(fontObserverMutex);
  if (--fontObserverEnvironments != 0) return;
  CFNotificationCenterRemoveObserver(CFNotificationCenterGetLocalCenter(), &fontRegistryVersion,
                                     kCTFontManagerRegisteredFontsChangedNotification, nullptr);
  CFNotificationCenterRemoveObserver(CFNotificationCenterGetDistributedCenter(), &fontRegistryVersion,
                                     kCTFontManagerRegisteredFontsChangedNotification, nullptr);
}
static void ObserveFontRegistry(FontObserverEnvironment *environment) {
  if (environment->watching) return;
  std::lock_guard<std::mutex> lock(fontObserverMutex);
  if (fontObserverEnvironments++ == 0) {
    for (CFNotificationCenterRef center : {CFNotificationCenterGetLocalCenter(), CFNotificationCenterGetDistributedCenter()}) {
      CFNotificationCenterAddObserver(center, &fontRegistryVersion, FontRegistryChanged,
                                     kCTFontManagerRegisteredFontsChangedNotification, nullptr,
                                     CFNotificationSuspensionBehaviorDeliverImmediately);
    }
  }
  environment->watching = true;
}
static napi_value FontRegistryVersion(napi_env env, napi_callback_info info) {
  napi_value result;
  // Font IPC runs on the application thread, whose run loop delivers session
  // notifications. Other Node workers retain uncached enumeration behavior.
  if (![NSThread isMainThread]) { napi_get_null(env, &result); return result; }
  void *data;
  napi_get_cb_info(env, info, nullptr, nullptr, nullptr, &data);
  ObserveFontRegistry(static_cast<FontObserverEnvironment *>(data));
  napi_create_uint32(env, fontRegistryVersion.load(std::memory_order_relaxed), &result);
  return result;
}

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

// File work runs off the Electron main thread. Clones own their metadata and
// diverge on write; never substitute hard links for recovery snapshots.
struct FileWork {
  napi_async_work work;
  napi_deferred deferred;
  std::string source, destination, operation;
  int fd = -1, error = 0;
  bool supported = true;
};
static void PerformFileWork(napi_env env, void *data) {
  FileWork *job = static_cast<FileWork *>(data);
  int result;
  if (job->operation == "clone") result = clonefile(job->source.c_str(), job->destination.c_str(), CLONE_NOFOLLOW);
  else if (job->operation == "swap") result = renamex_np(job->source.c_str(), job->destination.c_str(), RENAME_SWAP);
  else if (job->operation == "install") result = renamex_np(job->source.c_str(), job->destination.c_str(), RENAME_EXCL);
  else {
    result = fcntl(job->fd, F_FULLFSYNC);
    if (result && (errno == EINVAL || errno == ENOTSUP)) result = fsync(job->fd);
    if (result) job->error = errno;
    close(job->fd); job->fd = -1;
    return;
  }
  if (result) {
    if (errno == EXDEV || errno == ENOTSUP || errno == ENOSYS) job->supported = false;
    else job->error = errno;
  }
}
static void FileWorkComplete(napi_env env, napi_status status, void *data) {
  FileWork *job = static_cast<FileWork *>(data);
  napi_value value;
  if (status != napi_ok || job->error) {
    napi_value message, code;
    napi_create_string_utf8(env, job->error ? strerror(job->error) : "File operation cancelled", NAPI_AUTO_LENGTH, &message);
    napi_create_error(env, nullptr, message, &value);
    const char *name = job->error == EEXIST ? "EEXIST" : job->error == ENOENT ? "ENOENT" : job->error == ENOSPC ? "ENOSPC" : "EIO";
    napi_create_string_utf8(env, name, NAPI_AUTO_LENGTH, &code);
    napi_set_named_property(env, value, "code", code);
    napi_reject_deferred(env, job->deferred, value);
  } else {
    napi_get_boolean(env, job->supported, &value);
    napi_resolve_deferred(env, job->deferred, value);
  }
  if (job->fd >= 0) close(job->fd);
  napi_delete_async_work(env, job->work);
  delete job;
}
static napi_value FileOperation(napi_env env, napi_callback_info info) {
  size_t argc = 3; napi_value args[3];
  napi_get_cb_info(env, info, &argc, args, nullptr, nullptr);
  auto stringArg = [&](size_t index, std::string &out) {
    size_t length = 0;
    if (index >= argc || napi_get_value_string_utf8(env, args[index], nullptr, 0, &length) != napi_ok || length > 16384) return false;
    std::vector<char> chars(length + 1);
    napi_get_value_string_utf8(env, args[index], chars.data(), chars.size(), &length);
    out.assign(chars.data(), length);
    return out.find('\0') == std::string::npos;
  };
  FileWork *job = new FileWork{};
  bool valid = stringArg(0, job->operation);
  if (valid && job->operation == "sync") {
    int fd;
    valid = argc > 1 && napi_get_value_int32(env, args[1], &fd) == napi_ok;
    if (valid) { job->fd = dup(fd); valid = job->fd >= 0; }
  } else valid = valid && (job->operation == "clone" || job->operation == "swap" || job->operation == "install") && stringArg(1, job->source) && stringArg(2, job->destination);
  if (!valid) { delete job; napi_throw_type_error(env, nullptr, "Invalid file operation"); return nullptr; }
  napi_value promise, name;
  napi_create_promise(env, &job->deferred, &promise);
  napi_create_string_utf8(env, "projectFileOperation", NAPI_AUTO_LENGTH, &name);
  napi_create_async_work(env, nullptr, name, PerformFileWork, FileWorkComplete, job, &job->work);
  napi_queue_async_work(env, job->work);
  return promise;
}

// libsystem_sandbox SPI: 0 allowed, 1 denied (also for a pid that is gone).
extern "C" int sandbox_check(pid_t pid, const char *operation, int type, ...);
static const int SANDBOX_FILTER_GLOBAL_NAME = 2;
static const int SANDBOX_CHECK_NO_REPORT = 0x40000000;

// For each pid, whether its sandbox denies a mach-lookup of `name`. A process
// cannot leave its sandbox, so a per-command name marks everything it started.
static napi_value SandboxDeniesLookup(napi_env env, napi_callback_info info) {
  size_t argc = 2; napi_value args[2];
  napi_get_cb_info(env, info, &argc, args, nullptr, nullptr);
  uint32_t count = 0; size_t length = 0; bool isArray = false;
  if (argc < 2 || napi_is_array(env, args[0], &isArray) != napi_ok || !isArray
      || napi_get_array_length(env, args[0], &count) != napi_ok
      || napi_get_value_string_utf8(env, args[1], nullptr, 0, &length) != napi_ok || !length || length > 1024) {
    napi_throw_type_error(env, nullptr, "Expected pids and a service name");
    return nullptr;
  }
  std::vector<char> name(length + 1);
  napi_get_value_string_utf8(env, args[1], name.data(), name.size(), &length);
  napi_value result;
  napi_create_array_with_length(env, count, &result);
  for (uint32_t i = 0; i < count; i++) {
    napi_value element, denied;
    int32_t pid = 0;
    napi_get_element(env, args[0], i, &element);
    bool valid = napi_get_value_int32(env, element, &pid) == napi_ok && pid > 0;
    napi_get_boolean(env, valid && sandbox_check(pid, "mach-lookup", SANDBOX_FILTER_GLOBAL_NAME | SANDBOX_CHECK_NO_REPORT, name.data()) == 1, &denied);
    napi_set_element(env, result, i, denied);
  }
  return result;
}


// NSFileVersion is usable without NSDocument. The AppKit Versions browser is
// deliberately not exposed: it requires document/window-controller ownership.
struct VersionWork {
  napi_async_work work; napi_deferred deferred;
  std::string operation, path, contents, result, error;
};
static void PerformVersionWork(napi_env env, void *data) {
  VersionWork *job = static_cast<VersionWork *>(data);
  @autoreleasepool {
    @try {
      NSURL *url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:job->path.c_str()]];
      if (job->operation == "snapshot") {
        NSError *error = nil;
        NSURL *contents = [NSURL fileURLWithPath:[NSString stringWithUTF8String:job->contents.c_str()]];
        NSFileVersion *version = [NSFileVersion addVersionOfItemAtURL:url withContentsOfURL:contents options:0 error:&error];
        if (!version) job->error = [[error localizedDescription] UTF8String] ?: "macOS could not save a version";
        else job->result = "true";
      } else {
        NSMutableArray *versions = [NSMutableArray array];
        for (NSFileVersion *version in [NSFileVersion otherVersionsOfItemAtURL:url]) {
          if (version.URL.path && version.modificationDate) [versions addObject:@{ @"path": version.URL.path, @"at": @([version.modificationDate timeIntervalSince1970] * 1000) }];
        }
        NSData *json = [NSJSONSerialization dataWithJSONObject:versions options:0 error:nil];
        job->result = std::string(static_cast<const char *>(json.bytes), json.length);
      }
    } @catch (NSException *exception) { job->error = [[exception reason] UTF8String] ?: "macOS version operation failed"; }
  }
}
static void CompleteVersionWork(napi_env env, napi_status status, void *data) {
  VersionWork *job = static_cast<VersionWork *>(data); napi_value value;
  if (status != napi_ok || !job->error.empty()) {
    napi_value message; napi_create_string_utf8(env, job->error.empty() ? "Version operation cancelled" : job->error.c_str(), NAPI_AUTO_LENGTH, &message);
    napi_create_error(env, nullptr, message, &value); napi_reject_deferred(env, job->deferred, value);
  } else {
    napi_create_string_utf8(env, job->result.c_str(), job->result.size(), &value); napi_resolve_deferred(env, job->deferred, value);
  }
  napi_delete_async_work(env, job->work); delete job;
}
static napi_value FileVersions(napi_env env, napi_callback_info info) {
  size_t argc = 3; napi_value args[3]; napi_get_cb_info(env, info, &argc, args, nullptr, nullptr);
  std::string strings[3];
  for (size_t i = 0; i < argc; i++) {
    size_t length = 0;
    if (napi_get_value_string_utf8(env, args[i], nullptr, 0, &length) != napi_ok || length > 16384) {
      napi_throw_type_error(env, nullptr, "Expected version operation and file paths"); return nullptr;
    }
    std::vector<char> buffer(length + 1); napi_get_value_string_utf8(env, args[i], buffer.data(), buffer.size(), &length);
    strings[i] = std::string(buffer.data(), length);
  }
  if (argc < 2 || (strings[0] != "list" && strings[0] != "snapshot") || strings[1].empty()
      || (strings[0] == "snapshot" && (argc < 3 || strings[2].empty()))) {
    napi_throw_type_error(env, nullptr, "Invalid version operation"); return nullptr;
  }
  VersionWork *job = new VersionWork{}; job->operation = strings[0]; job->path = strings[1]; job->contents = strings[2];
  napi_value promise, name; napi_create_promise(env, &job->deferred, &promise);
  napi_create_string_utf8(env, "fileVersions", NAPI_AUTO_LENGTH, &name);
  napi_create_async_work(env, nullptr, name, PerformVersionWork, CompleteVersionWork, job, &job->work);
  napi_queue_async_work(env, job->work); return promise;
}

static napi_value Init(napi_env env, napi_value exports) {
  napi_value versions;
  napi_create_function(env, "fileVersions", NAPI_AUTO_LENGTH, FileVersions, nullptr, &versions);
  napi_set_named_property(env, exports, "fileVersions", versions);
  FontObserverEnvironment *fontEnvironment = new FontObserverEnvironment();
  napi_add_env_cleanup_hook(env, RemoveFontObservers, fontEnvironment);
  napi_value trigger;
  napi_create_function(env, "triggerAlignment", NAPI_AUTO_LENGTH,
                       TriggerAlignment, nullptr, &trigger);
  napi_set_named_property(env, exports, "triggerAlignment", trigger);
  napi_value fonts;
  napi_create_function(env, "fontFamilies", NAPI_AUTO_LENGTH, FontFamilies, nullptr, &fonts);
  napi_set_named_property(env, exports, "fontFamilies", fonts);
  napi_value fontVersion;
  napi_create_function(env, "fontRegistryVersion", NAPI_AUTO_LENGTH, FontRegistryVersion, fontEnvironment, &fontVersion);
  napi_set_named_property(env, exports, "fontRegistryVersion", fontVersion);
  napi_value cloud;
  napi_create_function(env, "cloudFileState", NAPI_AUTO_LENGTH, CloudFileState, nullptr, &cloud);
  napi_set_named_property(env, exports, "cloudFileState", cloud);
  napi_value file;
  napi_create_function(env, "fileOperation", NAPI_AUTO_LENGTH, FileOperation, nullptr, &file);
  napi_set_named_property(env, exports, "fileOperation", file);
  napi_value sandbox;
  napi_create_function(env, "sandboxDeniesLookup", NAPI_AUTO_LENGTH, SandboxDeniesLookup, nullptr, &sandbox);
  napi_set_named_property(env, exports, "sandboxDeniesLookup", sandbox);
  return exports;
}

NAPI_MODULE(NODE_GYP_MODULE_NAME, Init)
