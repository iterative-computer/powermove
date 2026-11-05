// Public Sparkle 2.10.1 APIs only. Constructed/used on the app's main run loop.
// No NSApplication creation, default Sparkle windows, secrets or network in tests.
#import <Foundation/Foundation.h>
#import <Sparkle/Sparkle.h>
#import <Security/Security.h>
#import <mach/machine.h>

static BOOL pmMatches(NSDictionary *release, NSDictionary *actual) {
    return [actual[@"version"] isEqualToString:release[@"version"]]
        && [actual[@"url"] isEqualToString:release[@"archiveUrl"]]
        && [actual[@"size"] unsignedLongLongValue] == [release[@"size"] unsignedLongLongValue]
        && [actual[@"signature"] isEqualToString:release[@"sparkleSignature"]]
        && ![actual[@"delta"] boolValue] && ![actual[@"informationOnly"] boolValue]
        && [actual[@"installationType"] isEqualToString:@"application"];
}
// Pure enclosure validation, shared with the linked callback contract tests.
int pm_sparkle_matches(const char *policy, const char *enclosure) {
    @autoreleasepool {
        if (!policy || !enclosure) return 0;
        id r = [NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:policy] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:NULL];
        id a = [NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:enclosure] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:NULL];
        if (![r isKindOfClass:NSDictionary.class] || ![a isKindOfClass:NSDictionary.class]) return 0;
        return pmMatches(r, a);
    }
}

@interface PMPolicyDriver : NSObject <SPUUserDriver, SPUUpdaterDelegate>
@property(nonatomic, strong) SPUUpdater *updater;
@property(nonatomic, strong) NSDictionary *policy;
@property(nonatomic, strong) NSMutableArray<NSNumber *> *events;
@property(nonatomic, copy) void (^found)(SPUUserUpdateChoice);
@property(nonatomic, copy) void (^ready)(SPUUserUpdateChoice);
@property(nonatomic, copy) void (^cancelDownload)(void);
@property(nonatomic) BOOL started;
@property(nonatomic) BOOL extracting;
@property(nonatomic) BOOL confirmedCancel;
@end
@implementation PMPolicyDriver
- (void)emit:(int)event { [self.events addObject:@(event)]; }
- (BOOL)matches:(SUAppcastItem *)item {
    id enclosure = item.propertiesDictionary[@"enclosure"];
    if (![enclosure isKindOfClass:NSDictionary.class] || !item.fileURL) return NO;
    return pmMatches(self.policy, @{@"version":item.versionString, @"url":item.fileURL.absoluteString,
        @"size":@(item.contentLength), @"signature":enclosure[@"sparkle:edSignature"] ?: @"",
        @"delta":@(item.isDeltaUpdate), @"informationOnly":@(item.informationOnlyUpdate),
        @"installationType":item.installationType ?: @""});
}
- (BOOL)updater:(SPUUpdater *)u mayPerformUpdateCheck:(SPUUpdateCheck)c error:(NSError **)error { return self.policy != nil; }
- (NSSet<NSString *> *)allowedChannelsForUpdater:(SPUUpdater *)u {
    return [self.policy[@"channel"] isEqualToString:@"beta"] ? [NSSet setWithObject:@"beta"] : [NSSet set];
}
- (NSString *)feedURLStringForUpdater:(SPUUpdater *)u { return u.hostBundle.infoDictionary[@"SUFeedURL"]; }
- (BOOL)updaterShouldPromptForPermissionToCheckForUpdates:(SPUUpdater *)u { return NO; }
- (BOOL)updater:(SPUUpdater *)u shouldDownloadReleaseNotesForUpdate:(SUAppcastItem *)i { return NO; }
- (SUAppcastItem *)bestValidUpdateInAppcast:(SUAppcast *)appcast forUpdater:(SPUUpdater *)u {
    for (SUAppcastItem *i in appcast.items) { if ([self matches:i]) return i; }
    return nil;
}
- (BOOL)updater:(SPUUpdater *)u shouldProceedWithUpdate:(SUAppcastItem *)i updateCheck:(SPUUpdateCheck)c error:(NSError **)e { return [self matches:i]; }
- (void)updater:(SPUUpdater *)u willDownloadUpdate:(SUAppcastItem *)i withRequest:(NSMutableURLRequest *)r {
    // Reject delta substitution before download. Sparkle still verifies archive
    // EdDSA + app signature/DR. Never accept a newer/different enclosure.
    if (![self matches:i]) { if (self.cancelDownload) self.cancelDownload(); [self emit:6]; }
}
- (void)showUpdatePermissionRequest:(SPUUpdatePermissionRequest *)r reply:(void (^)(SUUpdatePermissionResponse *))reply {
    reply([[SUUpdatePermissionResponse alloc] initWithAutomaticUpdateChecks:NO automaticUpdateDownloading:@NO sendSystemProfile:NO]);
}
- (void)showUserInitiatedUpdateCheckWithCancellation:(void (^)(void))c { self.cancelDownload = c; }
- (void)showUpdateFoundWithAppcastItem:(SUAppcastItem *)i state:(SPUUserUpdateState *)s reply:(void (^)(SPUUserUpdateChoice))r {
    // A prior extracted installation is never inferred safe from cached state.
    if (![self matches:i] || s.stage != SPUUserUpdateStageNotDownloaded) { r(SPUUserUpdateChoiceSkip); [self emit:6]; return; }
    self.found = r; [self emit:1];
}
- (void)showUpdateReleaseNotesWithDownloadData:(SPUDownloadData *)d {}
- (void)showUpdateReleaseNotesFailedToDownloadWithError:(NSError *)e {}
- (void)showUpdateNotFoundWithError:(NSError *)e acknowledgement:(void (^)(void))a { [self emit:6]; a(); }
- (void)showUpdaterError:(NSError *)e acknowledgement:(void (^)(void))a { [self emit:6]; a(); }
- (void)showDownloadInitiatedWithCancellation:(void (^)(void))c { self.cancelDownload = c; [self emit:2]; }
- (void)showDownloadDidReceiveExpectedContentLength:(uint64_t)n {
    if (n != [self.policy[@"size"] unsignedLongLongValue]) { if (self.cancelDownload) self.cancelDownload(); [self emit:6]; }
}
- (void)showDownloadDidReceiveDataOfLength:(uint64_t)n {}
- (void)showDownloadDidStartExtractingUpdate { self.extracting = YES; self.cancelDownload = nil; [self emit:3]; }
- (void)showExtractionReceivedProgress:(double)p {}
- (void)showReadyToInstallAndRelaunch:(void (^)(SPUUserUpdateChoice))r { self.ready = r; [self emit:4]; }
- (void)showInstallingUpdateWithApplicationTerminated:(BOOL)t retryTerminatingApplication:(void (^)(void))retry {}
- (void)showUpdateInstalledAndRelaunched:(BOOL)r acknowledgement:(void (^)(void))a { [self emit:5]; a(); }
- (void)dismissUpdateInstallation {
    self.found = nil; self.ready = nil; self.cancelDownload = nil;
    if (!self.extracting || self.confirmedCancel) [self emit:7];
}
- (void)showUpdateInFocus {}
@end

void *pm_sparkle_create(const char *bundlePath) {
    if (!NSThread.isMainThread || !bundlePath) return NULL;
    NSBundle *bundle = [NSBundle bundleWithPath:[NSString stringWithUTF8String:bundlePath]];
    // Never override or invent endpoint/key configuration from the renderer.
    NSDictionary *info = bundle.infoDictionary;
    NSString *feed = info[@"SUFeedURL"];
    NSURL *url = [feed isKindOfClass:NSString.class] ? [NSURL URLWithString:feed] : nil;
    if (!bundle || ![url.scheme isEqualToString:@"https"] || !url.host || url.user || url.password || url.query || url.fragment
        || ![info[@"SUPublicEDKey"] isKindOfClass:NSString.class]
        || [info[@"SUEnableAutomaticChecks"] boolValue] || [info[@"SUAutomaticallyUpdate"] boolValue]) return NULL;
    PMPolicyDriver *d = [PMPolicyDriver new]; d.events = [NSMutableArray new];
    d.updater = [[SPUUpdater alloc] initWithHostBundle:bundle applicationBundle:bundle userDriver:d delegate:d];
    return (__bridge_retained void *)d;
}
int pm_sparkle_check(void *context, const char *json) {
    if (!NSThread.isMainThread || !context || !json) return 0;
    PMPolicyDriver *d = (__bridge PMPolicyDriver *)context;
    if (d.updater.sessionInProgress) return 0;
    NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
    id release = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    if (![release isKindOfClass:NSDictionary.class]) return 0;
    if (![release[@"bundleId"] isEqualToString:d.updater.hostBundle.bundleIdentifier]) return 0;
    d.policy = release;
    if (!d.started) { if (![d.updater startUpdater:NULL]) return 0; d.started = YES; }
    d.updater.automaticallyChecksForUpdates = NO;
    d.updater.automaticallyDownloadsUpdates = NO;
    [d.updater checkForUpdates]; return 1;
}
int pm_sparkle_action(void *context, int action) {
    if (!NSThread.isMainThread || !context) return 0;
    PMPolicyDriver *d = (__bridge PMPolicyDriver *)context;
    if (action == 1 && d.found) { void (^reply)(SPUUserUpdateChoice) = d.found; d.found = nil; reply(SPUUserUpdateChoiceInstall); return 1; }
    if (action == 2 && d.ready) { void (^reply)(SPUUserUpdateChoice) = d.ready; d.ready = nil; reply(SPUUserUpdateChoiceInstall); return 1; }
    if (action == 3) {
        if (d.found) { d.found(SPUUserUpdateChoiceDismiss); d.found = nil; }
        if (d.ready) { d.confirmedCancel = YES; d.ready(SPUUserUpdateChoiceSkip); d.ready = nil; }
        if (d.cancelDownload) { d.cancelDownload(); d.cancelDownload = nil; }
        // Extraction cannot be cancelled through a public cancellation method.
        // The application's termination guard MUST remain held through Ready.
        return 1;
    }
    return 0;
}
int pm_sparkle_poll(void *context) {
    if (!NSThread.isMainThread || !context) return -1;
    PMPolicyDriver *d = (__bridge PMPolicyDriver *)context;
    if (!d.events.count) return 0;
    int event = d.events.firstObject.intValue; [d.events removeObjectAtIndex:0]; return event;
}
// Do not drop an active driver: Sparkle's helpers can outlive the object.
// The Rust owner retains it for the application's lifetime once armed.
void pm_sparkle_release(void *context) { if (NSThread.isMainThread && context) { PMPolicyDriver *value = CFBridgingRelease(context); value.updater = nil; } }

int pm_sparkle_installed(const char *path, const char *version, const char *identifier, const char *architecture, const char *requirement) {
    @autoreleasepool {
        if (!path || !version || !identifier || !architecture || !requirement) return 0;
        NSURL *url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:path]];
        NSBundle *bundle = [NSBundle bundleWithURL:url];
        if (![bundle.bundleIdentifier isEqualToString:[NSString stringWithUTF8String:identifier]]
            || ![bundle.infoDictionary[@"CFBundleVersion"] isEqualToString:[NSString stringWithUTF8String:version]]) return 0;
        NSString *arch = [NSString stringWithUTF8String:architecture];
        NSNumber *cpu = [arch isEqualToString:@"arm64"] ? @(CPU_TYPE_ARM64) : [arch isEqualToString:@"x86_64"] ? @(CPU_TYPE_X86_64) : nil;
        if (!cpu || ![bundle.executableArchitectures containsObject:cpu]) return 0;
        SecStaticCodeRef code = NULL; SecRequirementRef dr = NULL;
        OSStatus result = SecStaticCodeCreateWithPath((__bridge CFURLRef)url, kSecCSDefaultFlags, &code);
        if (result == errSecSuccess) result = SecRequirementCreateWithString((__bridge CFStringRef)[NSString stringWithUTF8String:requirement], kSecCSDefaultFlags, &dr);
        if (result == errSecSuccess) result = SecStaticCodeCheckValidity(code, kSecCSCheckAllArchitectures | kSecCSStrictValidate | kSecCSCheckNestedCode, dr);
        if (code) CFRelease(code); if (dr) CFRelease(dr);
        return result == errSecSuccess;
    }
}

const char *pm_sparkle_public_key(void *context) {
    if (!NSThread.isMainThread || !context) return NULL;
    PMPolicyDriver *d = (__bridge PMPolicyDriver *)context;
    return [d.updater.hostBundle.infoDictionary[@"SUPublicEDKey"] UTF8String];
}
