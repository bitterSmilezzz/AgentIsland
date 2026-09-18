import AppKit
import Carbon

// MARK: - 全局系统级快捷键管理器（基于 Carbon HIToolbox，免辅助功能授权）

public final class GlobalHotKeyManager {
    public static let shared = GlobalHotKeyManager()

    public var onToggle: (() -> Void)?

    private var hotKeyRef: EventHotKeyRef?
    private var eventHandlerRef: EventHandlerRef?

    public private(set) var isRegistered = false

    private init() {}

    deinit {
        unregister()
    }

    public func register() {
        guard !isRegistered else { return }

        var eventType = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let selfPtr = Unmanaged.passUnretained(self).toOpaque()

        let status = InstallEventHandler(GetApplicationEventTarget(), { _, event, userData -> OSStatus in
            guard let userData = userData else { return OSStatus(eventNotHandledErr) }
            let mgr = Unmanaged<GlobalHotKeyManager>.fromOpaque(userData).takeUnretainedValue()
            DispatchQueue.main.async {
                mgr.onToggle?()
            }
            return noErr
        }, 1, &eventType, selfPtr, &eventHandlerRef)

        guard status == noErr else { return }

        // 默认热键：⌥ + A (Option + A)
        let hotKeyID = EventHotKeyID(signature: OSType(0x4149_534C), id: 1) // "AISL"
        let modifiers = UInt32(optionKey)
        let registerStatus = RegisterEventHotKey(UInt32(kVK_ANSI_A), modifiers, hotKeyID, GetApplicationEventTarget(), 0, &hotKeyRef)

        if registerStatus == noErr {
            isRegistered = true
        }
    }

    public func unregister() {
        if let ref = hotKeyRef {
            UnregisterEventHotKey(ref)
            hotKeyRef = nil
        }
        if let ref = eventHandlerRef {
            RemoveEventHandler(ref)
            eventHandlerRef = nil
        }
        isRegistered = false
    }

    public func setEnabled(_ enabled: Bool) {
        if enabled {
            register()
        } else {
            unregister()
        }
    }
}
