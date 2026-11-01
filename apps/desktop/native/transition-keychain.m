// Legacy Keychain ACL handoff. Build/sign only on macOS; no shell or secret argv.
#import <Foundation/Foundation.h>
#import <Security/Security.h>
#include <unistd.h>
static void require(BOOL ok) { if (!ok) @throw [NSException exceptionWithName:@"HandoffFailed" reason:@"Keychain handoff failed" userInfo:nil]; }
static void status(OSStatus code) { require(code == errSecSuccess); }
static BOOL sameTrustedApps(SecAccessRef access, CFArrayRef expected) {
  CFArrayRef acls = NULL; if (SecAccessCopyACLList(access, &acls)) return NO;
  BOOL found = NO;
  for (CFIndex i = 0; i < CFArrayGetCount(acls); i++) {
    SecACLRef acl = (SecACLRef)CFArrayGetValueAtIndex(acls, i);
    CSSM_ACL_AUTHORIZATION_TAG auth[32]; uint32 count = 32;
    if (SecACLGetAuthorizations(acl, auth, &count)) continue;
    BOOL decrypt = NO;
    for (uint32 j = 0; j < count; j++) if (auth[j] == CSSM_ACL_AUTHORIZATION_DECRYPT) decrypt = YES;
    if (!decrypt) continue;
    CFArrayRef apps = NULL; CFStringRef description = NULL; CSSM_ACL_KEYCHAIN_PROMPT_SELECTOR prompt;
    if (SecACLCopyContents(acl, &apps, &description, &prompt)) continue;
    BOOL match = apps && CFArrayGetCount(apps) == 2;
    for (CFIndex j = 0; match && j < 2; j++) {
      CFDataRef actual = NULL; BOOL has = NO;
      if (SecTrustedApplicationCopyData((SecTrustedApplicationRef)CFArrayGetValueAtIndex(apps, j), &actual)) match = NO;
      for (CFIndex k = 0; actual && k < 2; k++) {
        CFDataRef wanted = NULL;
        if (!SecTrustedApplicationCopyData((SecTrustedApplicationRef)CFArrayGetValueAtIndex(expected, k), &wanted)) {
          if (CFEqual(actual, wanted)) has = YES;
          CFRelease(wanted);
        }
      }
      if (actual) CFRelease(actual);
      if (!has) match = NO;
    }
    // Any broader decrypt ACL makes the item ineligible.
    if (apps) CFRelease(apps); if (description) CFRelease(description);
    if (!match) { CFRelease(acls); return NO; }
    found = YES;
  }
  CFRelease(acls); return found;
}
int main(void) { @autoreleasepool { @try {
  NSMutableData *input = [NSMutableData data]; uint8_t buffer[4096]; ssize_t n;
  while ((n = read(STDIN_FILENO, buffer, sizeof buffer)) > 0) { require(input.length + (NSUInteger)n < 48 * 1024 * 1024); [input appendBytes:buffer length:(NSUInteger)n]; }
  require(n == 0);
  NSDictionary *request = [NSJSONSerialization JSONObjectWithData:input options:0 error:nil]; require([request isKindOfClass:NSDictionary.class]);
  NSArray *paths = request[@"applications"], *requirements = request[@"designatedRequirements"];
  require([paths isKindOfClass:NSArray.class] && paths.count == 2 && [requirements isKindOfClass:NSArray.class] && requirements.count == 2);
  require(![requirements[0] isEqual:requirements[1]]);
  NSMutableArray *trusted = [NSMutableArray array];
  for (NSUInteger i = 0; i < 2; i++) {
    require([paths[i] isKindOfClass:NSString.class] && [paths[i] isAbsolutePath] && [requirements[i] isKindOfClass:NSString.class]);
    SecStaticCodeRef code = NULL; SecRequirementRef expected = NULL, actual = NULL;
    status(SecStaticCodeCreateWithPath((__bridge CFURLRef)[NSURL fileURLWithPath:paths[i]], kSecCSDefaultFlags, &code));
    status(SecRequirementCreateWithString((__bridge CFStringRef)requirements[i], kSecCSDefaultFlags, &expected));
    status(SecStaticCodeCheckValidity(code, kSecCSStrictValidate | kSecCSCheckAllArchitectures, expected));
    status(SecCodeCopyDesignatedRequirement(code, kSecCSDefaultFlags, &actual));
    CFDataRef a = NULL, b = NULL;
    status(SecRequirementCopyData(actual, kSecCSDefaultFlags, &a)); status(SecRequirementCopyData(expected, kSecCSDefaultFlags, &b));
    require(CFEqual(a, b)); CFRelease(a); CFRelease(b); CFRelease(actual); CFRelease(expected); CFRelease(code);
    SecTrustedApplicationRef app = NULL; status(SecTrustedApplicationCreateFromPath([paths[i] fileSystemRepresentation], &app));
    [trusted addObject:CFBridgingRelease(app)];
  }
  NSString *service = request[@"service"], *account = request[@"account"];
  require([service isEqual:@"Powermove Native Legacy Handoff"] && [account isKindOfClass:NSString.class] && account.length == 64);
  NSMutableData *secret = [[[NSData alloc] initWithBase64EncodedString:request[@"value"] options:0] mutableCopy]; require(secret != nil);
  NSData *serviceData = [service dataUsingEncoding:NSUTF8StringEncoding], *accountData = [account dataUsingEncoding:NSUTF8StringEncoding];
  SecKeychainAttribute attrs[] = {{kSecServiceItemAttr, (UInt32)serviceData.length, (void *)serviceData.bytes}, {kSecAccountItemAttr, (UInt32)accountData.length, (void *)accountData.bytes}};
  SecKeychainAttributeList list = {2, attrs}; SecAccessRef access = NULL;
  status(SecAccessCreate(CFSTR("Powermove Native Legacy Handoff"), (__bridge CFArrayRef)trusted, &access));
  SecKeychainItemRef item = NULL;
  OSStatus result = SecKeychainItemCreateFromContent(kSecGenericPasswordItemClass, &list, (UInt32)secret.length, secret.bytes, NULL, access, &item);
  CFRelease(access);
  if (result == errSecDuplicateItem) {
    UInt32 length = 0; void *data = NULL;
    status(SecKeychainFindGenericPassword(NULL, (UInt32)serviceData.length, serviceData.bytes, (UInt32)accountData.length, accountData.bytes, &length, &data, &item));
    BOOL equal = length == secret.length && memcmp(data, secret.bytes, length) == 0;
    if (data) { memset(data, 0, length); SecKeychainItemFreeContent(NULL, data); }
    require(equal);
  } else status(result);
  memset(secret.mutableBytes, 0, secret.length); memset(input.mutableBytes, 0, input.length);
  SecAccessRef actualAccess = NULL; status(SecKeychainItemCopyAccess(item, &actualAccess));
  require(sameTrustedApps(actualAccess, (__bridge CFArrayRef)trusted)); CFRelease(actualAccess);
  CFDataRef ref = NULL; status(SecKeychainItemCreatePersistentReference(item, &ref)); CFRelease(item);
  NSDictionary *receipt = @{@"persistentRef": [(__bridge NSData *)ref base64EncodedStringWithOptions:0], @"designatedRequirements": requirements};
  NSData *output = [NSJSONSerialization dataWithJSONObject:receipt options:0 error:nil]; CFRelease(ref);
  require(write(STDOUT_FILENO, output.bytes, output.length) == (ssize_t)output.length); return 0;
} @catch (__unused NSException *exception) { fputs("Keychain handoff failed\n", stderr); return 1; } } }
