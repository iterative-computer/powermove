// Public Sparkle 2.10.1 API only. No UI, external archive adoption, extraction,
// signature bypass or forced termination. Host supplies an atomic guard probe.
#import <Foundation/Foundation.h>
#import <Sparkle/Sparkle.h>
#include <stdbool.h>
#include <stdint.h>

typedef bool (*PMEvent)(void *, const uint8_t *, size_t);
typedef bool (*PMPermit)(void *);
typedef void (*PMRelease)(void *);

@interface PMSparkle : NSObject <SPUUserDriver, SPUUpdaterDelegate>
@property void *context;
@property PMEvent event;
@property PMPermit permit;
@property PMRelease releaseContext;
@property(nonatomic, strong) SPUUpdater *updater;
@property(nonatomic, copy) NSString *operation;
@property(nonatomic, copy) NSDictionary *policy;
@property(nonatomic, copy) void (^foundReply)(SPUUserUpdateChoice);
@property(nonatomic, copy) void (^readyReply)(SPUUserUpdateChoice);
@property(nonatomic, copy) void (^cancellation)(void);
@property BOOL closing;
@property BOOL recovering;
@end

@implementation PMSparkle
- (void)dealloc { if (_releaseContext) _releaseContext(_context); }
- (BOOL)emit:(NSString *)kind enclosure:(NSDictionary *)enclosure {
    NSMutableDictionary *value = [@{@"kind":kind, @"id":self.operation ?: @""} mutableCopy];
    if (enclosure) value[@"enclosure"] = enclosure;
    NSData *bytes = [NSJSONSerialization dataWithJSONObject:value options:0 error:NULL];
    return bytes && self.event(self.context, bytes.bytes, bytes.length);
}
- (BOOL)allowed {
    return !self.closing && self.permit(self.context)
        && [NSDate date].timeIntervalSince1970 < [self.policy[@"expiresUnix"] doubleValue];
}
- (NSDictionary *)enclosure:(SUAppcastItem *)item {
    // propertiesDictionary is public. Pin the upstream RSS enclosure attribute
    // spelling instead of accessing Sparkle's private SUSignatures class.
    id signature = item.propertiesDictionary[@"enclosure"][@"sparkle:edSignature"];
    return @{@"version":item.versionString ?: @"", @"url":item.fileURL.absoluteString ?: @"",
             @"bytes":@(item.contentLength), @"ed_signature":[signature isKindOfClass:NSString.class] ? signature : @""};
}
- (BOOL)matches:(SUAppcastItem *)item {
    NSDictionary *e = [self enclosure:item];
    return !item.deltaUpdate && !item.informationOnlyUpdate
        && [item.installationType isEqualToString:@"application"]
        && [e[@"version"] isEqual:self.policy[@"version"]]
        && [e[@"url"] isEqual:self.policy[@"archiveUrl"]]
        && [e[@"bytes"] isEqual:self.policy[@"size"]]
        && [e[@"ed_signature"] isEqual:self.policy[@"sparkleSignature"]];
}
- (void)cancel {
    // Unknown persisted installer state is evidence for recovery, never an
    // instruction to discard that state or infer installation success.
    if (self.recovering) return;
    if (self.cancellation) { void (^cancel)(void) = self.cancellation; self.cancellation = nil; cancel(); }
    if (self.foundReply) { void (^reply)(SPUUserUpdateChoice) = self.foundReply; self.foundReply = nil; reply(SPUUserUpdateChoiceSkip); }
    if (self.readyReply) { void (^reply)(SPUUserUpdateChoice) = self.readyReply; self.readyReply = nil; reply(SPUUserUpdateChoiceSkip); }
}
- (void)command:(NSDictionary *)command {
    NSAssert(NSThread.isMainThread, @"Sparkle commands require the main run loop");
    NSString *name = command[@"command"];
    if ([name isEqualToString:@"check"]) {
        if (self.operation || self.closing) { [self emit:@"aborted" enclosure:nil]; return; }
        self.operation = command[@"id"];
        self.policy = command[@"release"];
        if (![self allowed]) { [self emit:@"aborted" enclosure:nil]; return; }
        NSBundle *bundle = NSBundle.mainBundle;
        if (![[[NSBundle bundleForClass:SPUUpdater.class] objectForInfoDictionaryKey:@"CFBundleShortVersionString"] isEqual:@"2.10.1"]) {
            [self emit:@"aborted" enclosure:nil]; return;
        }
        // Reject disabled/mismatched bootstrap; no environment/feed fallback.
        NSURL *configURL = [bundle URLForResource:@"update-config" withExtension:@"json"];
        NSData *configBytes = configURL ? [NSData dataWithContentsOfURL:configURL] : nil;
        id parsed = configBytes ? [NSJSONSerialization JSONObjectWithData:configBytes options:0 error:NULL] : nil;
        NSDictionary *config = [parsed isKindOfClass:NSDictionary.class] ? parsed : nil;
        NSURL *feed = [NSURL URLWithString:[bundle objectForInfoDictionaryKey:@"SUFeedURL"] ?: @""];
        if (![config[@"enabled"] isEqual:@YES] || ![bundle.bundleIdentifier isEqual:self.policy[@"bundleId"]]
            || ![feed.scheme isEqualToString:@"https"] || !feed.host || feed.user || feed.password
            || feed.query || feed.fragment || ![bundle objectForInfoDictionaryKey:@"SUPublicEDKey"]) {
            [self emit:@"aborted" enclosure:nil]; return;
        }
        self.updater = [[SPUUpdater alloc] initWithHostBundle:bundle applicationBundle:bundle userDriver:self delegate:self];
        self.updater.automaticallyChecksForUpdates = NO;
        self.updater.automaticallyDownloadsUpdates = NO;
        self.updater.sendsSystemProfile = NO;
        NSError *error = nil;
        if (![self.updater startUpdater:&error]) { [self emit:@"aborted" enclosure:nil]; return; }
        [self.updater checkForUpdates];
        return;
    }
    if (![command[@"id"] isEqual:self.operation]) { [self emit:@"aborted" enclosure:nil]; return; }
    if ([name isEqualToString:@"cancel"] || ![self allowed]) { [self cancel]; return; }
    if ([name isEqualToString:@"download"] && self.foundReply) {
        void (^reply)(SPUUserUpdateChoice) = self.foundReply; self.foundReply = nil;
        reply(SPUUserUpdateChoiceInstall);
    } else if ([name isEqualToString:@"install"] && self.readyReply) {
        void (^reply)(SPUUserUpdateChoice) = self.readyReply; self.readyReply = nil;
        reply(SPUUserUpdateChoiceInstall);
    } else { [self emit:@"aborted" enclosure:nil]; }
}
- (BOOL)updaterShouldPromptForPermissionToCheckForUpdates:(SPUUpdater *)updater { (void)updater; return NO; }
- (BOOL)updater:(SPUUpdater *)updater mayPerformUpdateCheck:(SPUUpdateCheck)check error:(NSError **)error {
    (void)updater; (void)check;
    if ([self allowed]) return YES;
    if (error) *error = [NSError errorWithDomain:@"Powermove.UpdatePolicy" code:1 userInfo:nil];
    return NO;
}
- (SUAppcastItem *)bestValidUpdateInAppcast:(SUAppcast *)appcast forUpdater:(SPUUpdater *)updater {
    (void)updater;
    for (SUAppcastItem *item in appcast.items) if ([self matches:item]) return item;
    return [SUAppcastItem emptyAppcastItem];
}
- (BOOL)updater:(SPUUpdater *)updater shouldProceedWithUpdate:(SUAppcastItem *)item updateCheck:(SPUUpdateCheck)check error:(NSError **)error {
    (void)updater; (void)check;
    if ([self allowed] && [self matches:item]) return YES;
    if (error) *error = [NSError errorWithDomain:@"Powermove.UpdatePolicy" code:2 userInfo:nil];
    return NO;
}
- (BOOL)updater:(SPUUpdater *)updater shouldDownloadReleaseNotesForUpdate:(SUAppcastItem *)item { (void)updater; (void)item; return NO; }
- (void)showUpdatePermissionRequest:(SPUUpdatePermissionRequest *)request reply:(void (^)(SUUpdatePermissionResponse *))reply {
    (void)request; reply([[SUUpdatePermissionResponse alloc] initWithAutomaticUpdateChecks:NO sendSystemProfile:NO]);
}
- (void)showUserInitiatedUpdateCheckWithCancellation:(void (^)(void))cancellation { self.cancellation = cancellation; }
- (void)showUpdateFoundWithAppcastItem:(SUAppcastItem *)item state:(SPUUserUpdateState *)state reply:(void (^)(SPUUserUpdateChoice))reply {
    if (state.stage != SPUUserUpdateStageNotDownloaded) {
        self.recovering = YES; self.foundReply = reply;
        [self emit:@"recovery_required" enclosure:nil]; return;
    }
    if (![self allowed] || ![self matches:item]) {
        reply(SPUUserUpdateChoiceSkip); [self emit:@"aborted" enclosure:nil]; return;
    }
    self.foundReply = reply;
    if (![self emit:@"found" enclosure:[self enclosure:item]]) [self cancel];
}
- (void)showUpdateReleaseNotesWithDownloadData:(SPUDownloadData *)data { (void)data; }
- (void)showUpdateReleaseNotesFailedToDownloadWithError:(NSError *)error { (void)error; }
- (void)showUpdateNotFoundWithError:(NSError *)error acknowledgement:(void (^)(void))acknowledgement {
    (void)error; [self emit:@"aborted" enclosure:nil]; acknowledgement();
}
- (void)showUpdaterError:(NSError *)error acknowledgement:(void (^)(void))acknowledgement {
    (void)error; [self emit:@"aborted" enclosure:nil]; acknowledgement();
}
- (void)showDownloadInitiatedWithCancellation:(void (^)(void))cancellation { self.cancellation = cancellation; if (![self allowed]) [self cancel]; }
- (void)showDownloadDidReceiveExpectedContentLength:(uint64_t)length { (void)length; }
- (void)showDownloadDidReceiveDataOfLength:(uint64_t)length { (void)length; if (![self allowed]) [self cancel]; }
- (void)showDownloadDidStartExtractingUpdate {
    self.cancellation = nil;
    if (![self emit:@"extracting" enclosure:nil]) self.closing = YES;
}
- (void)showExtractionReceivedProgress:(double)progress { (void)progress; }
- (void)showReadyToInstallAndRelaunch:(void (^)(SPUUserUpdateChoice))reply {
    self.readyReply = reply;
    if (![self allowed] || ![self emit:@"ready" enclosure:nil]) [self cancel];
}
- (void)showInstallingUpdateWithApplicationTerminated:(BOOL)terminated retryTerminatingApplication:(void (^)(void))retry {
    (void)terminated; (void)retry; // Never force/retry termination outside the host save barrier.
}
- (void)showUpdateInstalledAndRelaunched:(BOOL)relaunched acknowledgement:(void (^)(void))acknowledgement {
    (void)relaunched; [self emit:@"installed" enclosure:nil]; acknowledgement();
}
- (void)dismissUpdateInstallation { self.foundReply = nil; self.readyReply = nil; self.cancellation = nil; }
@end

void *pm_sparkle_new(void *context, PMEvent event, PMPermit permit, PMRelease release) {
    PMSparkle *bridge = [PMSparkle new];
    bridge.context = context; bridge.event = event; bridge.permit = permit; bridge.releaseContext = release;
    return (__bridge_retained void *)bridge;
}
void pm_sparkle_command(void *handle, const uint8_t *data, size_t length) {
    PMSparkle *bridge = (__bridge PMSparkle *)handle;
    NSData *bytes = [NSData dataWithBytes:data length:length];
    dispatch_async(dispatch_get_main_queue(), ^{
        NSDictionary *command = [NSJSONSerialization JSONObjectWithData:bytes options:0 error:NULL];
        if (command) [bridge command:command]; else [bridge emit:@"aborted" enclosure:nil];
    });
}
void pm_sparkle_drop(void *handle) {
    PMSparkle *bridge = (__bridge_transfer PMSparkle *)handle;
    dispatch_async(dispatch_get_main_queue(), ^{
        bridge.closing = YES; [bridge cancel]; bridge.updater = nil;
    });
}
