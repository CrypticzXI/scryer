import {
  BACKUP_INFO_FIELDS,
  JOB_RUN_FIELDS,
  MEDIA_SERVER_CONNECTION_FIELDS,
  PROVIDER_CONFIG_VALUE_FIELDS,
  SUBTITLE_PROVIDER_CONFIG_FIELDS,
  SUBTITLE_SETTINGS_FIELDS,
  TITLE_CORE_FIELDS,
} from "./queries";

const AUTH_USER_FIELDS = `
      id
      username
      hasPassword
      hasMfa
      hasPasskey
      accountKind
      appPermissions
      libraryPermissions {
        libraryId
        permissions
      }`;

const LOGIN_PAYLOAD_FIELDS = `
    token
    user {${AUTH_USER_FIELDS}
    }
    expiresAt
    mfaVerifiedUntil
    mfaEnrollmentRequired`;

export const loginMutation = `mutation Login($input: LoginInput!) {
  login(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const webauthnRegisterStartMutation = `mutation WebauthnRegisterStart {
  webauthnRegisterStart {
    challengeId
    optionsJson
  }
}`;

export const webauthnRegisterCompleteMutation = `mutation WebauthnRegisterComplete($input: WebauthnRegisterCompleteInput!) {
  webauthnRegisterComplete(input: $input) {
    id
    friendlyName
    createdAt
    lastUsedAt
  }
}`;

export const webauthnAuthenticateStartMutation = `mutation WebauthnAuthenticateStart($username: String) {
  webauthnAuthenticateStart(username: $username) {
    challengeId
    optionsJson
  }
}`;

export const webauthnAuthenticateCompleteMutation = `mutation WebauthnAuthenticateComplete($input: WebauthnCompleteInput!) {
  webauthnAuthenticateComplete(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const deleteMyPasskeyMutation = `mutation DeleteMyPasskey($id: ID!) {
  deleteMyPasskey(id: $id) {
    id
    deleted
  }
}`;

export const revokeMyOauthAppMutation = `mutation RevokeMyOauthApp($grantId: ID!) {
  revokeMyOauthApp(grantId: $grantId) {
    grantId
    revoked
  }
}`;

export const totpEnrollmentStartMutation = `mutation TotpEnrollmentStart {
  totpEnrollmentStart {
    challengeId
    otpauthUrl
    secretBase32
    expiresAt
  }
}`;

export const totpEnrollmentCompleteMutation = `mutation TotpEnrollmentComplete($input: TotpEnrollmentCompleteInput!) {
  totpEnrollmentComplete(input: $input) {
    status {
      enabled
      createdAt
      lastUsedAt
      recoveryCodesRemaining
    }
    recoveryCodes
  }
}`;

export const completeLoginMfaEnrollmentMutation = `mutation CompleteLoginMfaEnrollment($input: TotpEnrollmentCompleteInput!) {
  completeLoginMfaEnrollment(input: $input) {
    status {
      enabled
      createdAt
      lastUsedAt
      recoveryCodesRemaining
    }
    recoveryCodes
    login {
${LOGIN_PAYLOAD_FIELDS}
    }
  }
}`;

export const mfaVerifyStepUpMutation = `mutation MfaVerifyStepUp($input: TotpVerifyInput!) {
  mfaVerifyStepUp(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const totpDisableMutation = `mutation TotpDisable($input: TotpVerifyInput!) {
  totpDisable(input: $input) {
    enabled
    createdAt
    lastUsedAt
    recoveryCodesRemaining
  }
}`;

export const totpRegenerateRecoveryCodesMutation = `mutation TotpRegenerateRecoveryCodes($input: TotpVerifyInput!) {
  totpRegenerateRecoveryCodes(input: $input) {
    status {
      enabled
      createdAt
      lastUsedAt
      recoveryCodesRemaining
    }
    recoveryCodes
  }
}`;

export const createUserMutation = `mutation CreateUser($input: CreateUserInput!) {
  createUser(input: $input) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserPasswordMutation = `mutation SetUserPassword($input: SetUserPasswordInput!) {
  setUserPassword(input: $input) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserAppPermissionsMutation = `mutation SetUserAppPermissions($input: SetUserAppPermissionsInput!) {
  setUserAppPermissions(input: $input) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserLibraryPermissionsMutation = `mutation SetUserLibraryPermissions($input: SetUserLibraryPermissionsInput!) {
  setUserLibraryPermissions(input: $input) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const deleteUserMutation = `mutation DeleteUser($id: ID!) {
  deleteUser(id: $id) {
    id
    deleted
  }
}`;

export const resetUserMfaMutation = `mutation ResetUserMfa($id: ID!) {
  resetUserMfa(id: $id) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const deleteTitleMutation = `mutation DeleteTitle($input: DeleteTitleInput!) {
  deleteTitle(input: $input) {
    id
    deleted
  }
}`;

export const deleteTitlesMutation = `mutation DeleteTitles($input: DeleteTitlesInput!) {
  deleteTitles(input: $input) {
    acceptedTitleIds
    jobRun {
      id
      jobKey
      displayName
      category
      section
      status
      triggerSource
      startedAt
      completedAt
      summaryJson
      summaryText
      errorText
      progressJson
    }
  }
}`;

export const createIndexerMutation = `mutation CreateIndexer($input: CreateIndexerConfigInput!) {
  createIndexerConfig(input: $input) {
    id
    name
    providerType
    baseUrl
    hasApiKey
    storedSecretKeys
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    isManaged
    managedParentConfigId
    supportsManagedChildrenSync
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorAt
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    createdAt
    updatedAt
  }
}`;

export const updateIndexerMutation = `mutation UpdateIndexer($input: UpdateIndexerConfigInput!) {
  updateIndexerConfig(input: $input) {
    id
    name
    providerType
    baseUrl
    hasApiKey
    storedSecretKeys
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    isManaged
    managedParentConfigId
    supportsManagedChildrenSync
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorAt
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    createdAt
    updatedAt
  }
}`;

export const deleteIndexerMutation = `mutation DeleteIndexer($id: ID!) {
  deleteIndexerConfig(id: $id) {
    id
    deleted
  }
}`;

export const syncIndexerConfigMutation = `mutation SyncIndexerConfig($id: ID!) {
  syncIndexerConfig(id: $id) {
    parentConfigId
    createdIds
    updatedIds
    deletedIds
  }
}`;

export const testIndexerConnectionMutation = `mutation TestIndexerConnection($input: TestIndexerConnectionInput!) {
  testIndexerConnection(input: $input) {
    status
    message
    retryAfterSeconds
  }
}`;

export const createDownloadClientMutation = `mutation CreateDownloadClient($input: CreateDownloadClientConfigInput!) {
  createDownloadClientConfig(input: $input) {
    id
    name
    clientType
    baseUrl
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    isEnabled
    status
    lastError
    lastSeenAt
    createdAt
    updatedAt
  }
}`;

export const updateDownloadClientMutation = `mutation UpdateDownloadClient($input: UpdateDownloadClientConfigInput!) {
  updateDownloadClientConfig(input: $input) {
    id
    name
    clientType
    baseUrl
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    isEnabled
    status
    lastError
    lastSeenAt
    createdAt
    updatedAt
  }
}`;

export const testDownloadClientConnectionMutation = `mutation TestDownloadClientConnection($input: TestDownloadClientConnectionInput!) {
  testDownloadClientConnection(input: $input) {
    status
    message
    retryAfterSeconds
  }
}`;

export const deleteDownloadClientMutation = `mutation DeleteDownloadClient($id: ID!) {
  deleteDownloadClientConfig(id: $id) {
    id
    deleted
  }
}`;

export const reorderDownloadClientsMutation = `mutation ReorderDownloadClients($input: ReorderDownloadClientConfigsInput!) {
  reorderDownloadClientConfigs(input: $input) {
    ids
    reordered
  }
}`;

export const addTitleMutation = `mutation AddTitle($input: AddTitleInput!) {
  addTitle(input: $input) {
    title {${TITLE_CORE_FIELDS}
    }
    metadataHydrationState
    reusedExistingTitle
    reusedQueuedDownload
    downloadJobId
    queuedDownload {
      jobId
      titleId
      titleName
      sourceTitle
      sourceKind
    }
  }
}`;

export const addTitleAndQueueMutation = `mutation AddTitleAndQueue($input: AddTitleInput!) {
  addTitleAndQueueDownload(input: $input) {
    title {${TITLE_CORE_FIELDS}
    }
    metadataHydrationState
    reusedExistingTitle
    reusedQueuedDownload
    downloadJobId
    queuedDownload {
      jobId
      titleId
      titleName
      sourceTitle
      sourceKind
    }
  }
}`;

export const submitMediaRequestMutation = `mutation SubmitMediaRequest($input: SubmitMediaRequestInput!) {
  submitMediaRequest(input: $input) {
    accepted
  }
}`;

export const approveMediaRequestMutation = `mutation ApproveMediaRequest($input: ApproveMediaRequestInput!) {
  approveMediaRequest(input: $input) {
    accepted
    titleId
    wantedSearch {
      queuedCount
      skippedInProgressCount
    }
    searchError
  }
}`;

export const dismissMediaRequestMutation = `mutation DismissMediaRequest($requestId: ID!) {
  dismissMediaRequest(requestId: $requestId) {
    accepted
  }
}`;

export const updateMyMediaRequestMutation = `mutation UpdateMyMediaRequest($input: UpdateMediaRequestInput!) {
  updateMyMediaRequest(input: $input) {
    id
    libraryId
    facet
    status
    identityFingerprint
    title
    requestedQualityProfileId
    requestedQualityProfileName
    requestedMonitorType
    updatedAt
  }
}`;

export const cancelMyMediaRequestMutation = `mutation CancelMyMediaRequest($requestId: ID!) {
  cancelMyMediaRequest(requestId: $requestId) {
    accepted
  }
}`;

export const deleteMediaFileMutation = `mutation DeleteMediaFile($input: DeleteMediaFileInput!) {
  deleteMediaFile(input: $input) {
    id
    deleted
  }
}`;

export const scanLibraryMutation = `mutation ScanLibrary($input: ScanLibraryInput!) {
  scanLibrary(input: $input) {
    sessionId
    facet
    mode
    status
    startedAt
    updatedAt
  }
}`;

const LIBRARY_FIELDS = `
    id
    facet
    name
    slug
    isDefault
    roots {
      id
      path
      isDefault
    }`;

export const createLibraryMutation = `mutation CreateLibrary($input: CreateLibraryInput!) {
  createLibrary(input: $input) {${LIBRARY_FIELDS}
  }
}`;

export const updateLibraryMutation = `mutation UpdateLibrary($input: UpdateLibraryInput!) {
  updateLibrary(input: $input) {${LIBRARY_FIELDS}
  }
}`;

export const deleteLibraryMutation = `mutation DeleteLibrary($id: ID!) {
  deleteLibrary(id: $id) {
    id
    deleted
  }
}`;

export const cancelLibraryScanMutation = `mutation CancelLibraryScan($sessionId: ID!) {
  cancelLibraryScan(sessionId: $sessionId) {
    sessionId
    accepted
  }
}`;

export const scanTitleLibraryMutation = `mutation ScanTitleLibrary($titleId: ID!) {
  scanTitleLibrary(titleId: $titleId) {
    scanned
    matched
    imported
    skipped
    unmatched
  }
}`;

export const resolvePendingImportMutation = `mutation ResolvePendingImport($input: ResolvePendingImportInput!) {
  resolvePendingImport(input: $input) {
    created
    libraryScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    title {
      id
      name
      facet
      monitored
    }
  }
}`;

export const bindPendingImportMutation = `mutation BindPendingImport($input: BindPendingImportInput!) {
  bindPendingImport(input: $input) {
    created
    libraryScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    title {
      id
      name
      facet
      monitored
    }
  }
}`;

export const ignorePendingImportMutation = `mutation IgnorePendingImport($pendingImportId: ID!) {
  ignorePendingImport(pendingImportId: $pendingImportId) {
    id
    status
  }
}`;

export const triggerJobMutation = `mutation TriggerJob($jobKey: JobKeyValue!) {
  triggerJob(jobKey: $jobKey) {
${JOB_RUN_FIELDS}
  }
}`;

export const applyMediaRenameMutation = `mutation ApplyMediaRename($input: MediaRenameApplyInput!) {
  applyMediaRename(input: $input) {
    planFingerprint
    total
    applied
    skipped
    failed
    items {
      collectionId
      seriesMovieLinkIds
      currentPath
      proposedPath
      finalPath
      writeAction
      status
      reasonCode
      errorMessage
    }
  }
}`;

export const applyMediaRenameBulkMutation = `mutation ApplyMediaRenameBulk($input: MediaRenameBulkApplyInput!) {
  applyMediaRenameBulk(input: $input) {
    planFingerprint
    total
    applied
    skipped
    failed
    items {
      collectionId
      seriesMovieLinkIds
      currentPath
      proposedPath
      finalPath
      writeAction
      status
      reasonCode
      errorMessage
    }
  }
}`;

export const updateSubtitleSettingsMutation = `mutation UpdateSubtitleSettings($input: UpdateSubtitleSettingsInput!) {
  updateSubtitleSettings(input: $input) {${SUBTITLE_SETTINGS_FIELDS}
  }
}`;

export const createSubtitleProviderConfigMutation = `mutation CreateSubtitleProviderConfig($input: CreateSubtitleProviderConfigInput!) {
  createSubtitleProviderConfig(input: $input) {${SUBTITLE_PROVIDER_CONFIG_FIELDS}
  }
}`;

export const updateSubtitleProviderConfigMutation = `mutation UpdateSubtitleProviderConfig($input: UpdateSubtitleProviderConfigInput!) {
  updateSubtitleProviderConfig(input: $input) {${SUBTITLE_PROVIDER_CONFIG_FIELDS}
  }
}`;

export const deleteSubtitleProviderConfigMutation = `mutation DeleteSubtitleProviderConfig($id: ID!) {
  deleteSubtitleProviderConfig(id: $id) {
    id
    deleted
  }
}`;

export const testSubtitleProviderConnectionMutation = `mutation TestSubtitleProviderConnection($input: TestSubtitleProviderConnectionInput!) {
  testSubtitleProviderConnection(input: $input) {
    status
    message
    retryAfterSeconds
  }
}`;

export const updateAcquisitionSettingsMutation = `mutation UpdateAcquisitionSettings($input: UpdateAcquisitionSettingsInput!) {
  updateAcquisitionSettings(input: $input) {
    enabled
    upgradeCooldownHours
    sameTierMinDelta
    crossTierMinDelta
    forcedUpgradeDeltaBypass
    pollIntervalSeconds
    syncIntervalSeconds
    batchSize
  }
}`;

export const updateGeneralSettingsMutation = `mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
  updateGeneralSettings(input: $input) {
    keepHistoryForever
    historyRetentionDays
    pluginHttpCaBundlePem
    pluginHttpTrustedCertificates {
      fingerprintSha256
      pem
    }
  }
}`;

export const setMyUiSettingsMutation = `mutation SetMyUiSettings($input: SetMyUiSettingsInput!) {
  setMyUiSettings(input: $input) {
    theme
    dateTimeFormat
    highlightColor
    secondaryColor
    highContrastMode
    reduceMotion
    density
    sidebarMode
    defaultLandingView
    tableColumns {
      facet
      tableViewMode
      columnId
      columnOrder
      visible
    }
  }
}`;

export const clearTitleImageCacheMutation = `mutation ClearTitleImageCache {
  clearTitleImageCache {
    accepted
  }
}`;

export const createBackupMutation = `mutation CreateBackup($input: CreateBackupInput!) {
  createBackup(input: $input) {${BACKUP_INFO_FIELDS}
  }
}`;

export const prepareBackupDownloadMutation = `mutation PrepareBackupDownload($input: PrepareBackupDownloadInput!) {
  prepareBackupDownload(input: $input) {
    downloadUrl
    downloadAuthorizationToken
    expiresAt
  }
}`;

export const deleteBackupMutation = `mutation DeleteBackup($input: DeleteBackupInput!) {
  deleteBackup(input: $input) {
    filename
    deleted
  }
}`;

const AUTO_BACKUP_SETTINGS_FIELDS = `
    enabled
    dailyTimeLocal
    autoBackupKeyPresent
    autoBackupDisabledMissingKeyNotice
    nextRunAt`;

const BACKUP_SETTINGS_FIELDS = `
    customBackupPath
    defaultBackupPath
    effectiveBackupPath`;

export const updateAutoBackupSettingsMutation = `mutation UpdateAutoBackupSettings($input: UpdateAutoBackupSettingsInput!) {
  updateAutoBackupSettings(input: $input) {${AUTO_BACKUP_SETTINGS_FIELDS}
  }
}`;

export const updateBackupSettingsMutation = `mutation UpdateBackupSettings($input: UpdateBackupSettingsInput!) {
  updateBackupSettings(input: $input) {${BACKUP_SETTINGS_FIELDS}
  }
}`;

export const acknowledgeAutoBackupDisabledMissingKeyNoticeMutation = `mutation AcknowledgeAutoBackupDisabledMissingKeyNotice {
  acknowledgeAutoBackupDisabledMissingKeyNotice {${AUTO_BACKUP_SETTINGS_FIELDS}
  }
}`;

export const updateSecuritySettingsMutation = `mutation UpdateSecuritySettings($input: UpdateSecuritySettingsInput!) {
  updateSecuritySettings(input: $input) {
    formLoginEnabled
    passwordMinLength
    skipLoginForLocalIps
    mfaRequireConfigStepUp
    mfaRequirePasswordLogin
    totpRequireJellyfinLogin
    effectiveFormLoginEnabled
    envOverrideActive
    envOverrideDescription
  }
}`;

const LINKED_ACCOUNT_FIELDS = `
    id
    userId
    provider
    connectionId
    externalUserId
    username
    displayName
    avatarUrl
    status
    verifiedAt
    lastLoginAt
    createdAt
    updatedAt`;

export const createMediaServerConnectionMutation = `mutation CreateMediaServerConnection($input: CreateMediaServerConnectionInput!) {
  createMediaServerConnection(input: $input) {${MEDIA_SERVER_CONNECTION_FIELDS}
  }
}`;

export const updateMediaServerConnectionMutation = `mutation UpdateMediaServerConnection($input: UpdateMediaServerConnectionInput!) {
  updateMediaServerConnection(input: $input) {${MEDIA_SERVER_CONNECTION_FIELDS}
  }
}`;

export const deleteMediaServerConnectionMutation = `mutation DeleteMediaServerConnection($id: ID!) {
  deleteMediaServerConnection(id: $id) {
    id
    deleted
  }
}`;

export const testMediaServerConnectionMutation = `mutation TestMediaServerConnection($input: TestMediaServerConnectionInput!) {
  testMediaServerConnection(input: $input) {
    status
    message
    retryAfterSeconds
  }
}`;

export const discoverPlexMediaServersMutation = `mutation DiscoverPlexMediaServers($plexAuthToken: String!) {
  discoverPlexMediaServers(plexAuthToken: $plexAuthToken) {
    id
    name
  }
}`;

export const createExternalAccountInviteMutation = `mutation CreateExternalAccountInvite($input: CreateExternalAccountInviteInput!) {
  createExternalAccountInvite(input: $input) {${LINKED_ACCOUNT_FIELDS}
  }
}`;

export const linkPlexAccountMutation = `mutation LinkPlexAccount($input: LinkPlexAccountInput!) {
  linkPlexAccount(input: $input) {${LINKED_ACCOUNT_FIELDS}
  }
}`;

export const linkJellyfinAccountMutation = `mutation LinkJellyfinAccount($input: LinkJellyfinAccountInput!) {
  linkJellyfinAccount(input: $input) {${LINKED_ACCOUNT_FIELDS}
  }
}`;

export const unlinkExternalAccountMutation = `mutation UnlinkExternalAccount($linkedAccountId: ID!) {
  unlinkExternalAccount(linkedAccountId: $linkedAccountId) {
    linkedAccountId
    unlinked
  }
}`;

export const loginWithPlexMutation = `mutation LoginWithPlex($input: LoginWithPlexInput!) {
  loginWithPlex(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const loginWithJellyfinMutation = `mutation LoginWithJellyfin($input: LoginWithJellyfinInput!) {
  loginWithJellyfin(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const upsertDelayProfileMutation = `mutation UpsertDelayProfile($input: DelayProfileInput!) {
  upsertDelayProfile(input: $input) {
    id
    name
    usenetDelayMinutes
    torrentDelayMinutes
    preferredProtocol
    minAgeMinutes
    bypassScoreThreshold
    appliesToFacets
    tags
    priority
    enabled
  }
}`;

export const deleteDelayProfileMutation = `mutation DeleteDelayProfile($id: ID!) {
  deleteDelayProfile(id: $id) {
    id
  }
}`;

const qualityProfileCriteriaFields = `
      qualityTiers
      archivalQuality
      allowUnknownQuality
      sourceAllowlist
      sourceBlocklist
      videoCodecAllowlist
      videoCodecBlocklist
      audioCodecAllowlist
      audioCodecBlocklist
      dolbyVisionAllowed
      detectedHdrAllowed
      preferRemux
      allowBdDisk
      allowUpgrades
      scoringOverrides {
        allowX265Non4K
        blockDvWithoutFallback
        preferCompactEncodes
        preferLosslessAudio
        blockUpscaled
      }
      cutoffTier
      minScoreToGrab`;

const qualityProfileSettingsFieldSelection = `
    globalProfileId
    globalScoringPersona
    profiles {
      id
      name
      criteria {${qualityProfileCriteriaFields}
      }
    }
    categorySelections {
      scope
      overrideProfileId
      effectiveProfileId
      inheritsGlobal
    }
    categoryPersonaSelections {
      scope
      overridePersona
      effectivePersona
      inheritsGlobal
    }`;

const downloadClientRoutingFieldSelection = `
    clientId
    enabled
    category
    recentQueuePriority
    olderQueuePriority
    removeCompleted
    removeFailed`;

const indexerRoutingFieldSelection = `
    indexerId
    enabled
    categories
    priority`;

const mediaSettingsFieldSelection = `
    scope
    libraryPath
    rootFolders {
      path
      isDefault
    }
    requiredAudioLanguages
    renameEnabled
    renameTemplate
    renameCollisionPolicy
    renameMissingMetadataPolicy
    fillerPolicy
    recapPolicy
    monitorSpecials
    interSeasonMovies
    monitorFillerMovies
    nfoWriteOnImport
    plexmatchWriteOnImport
    importMode
    setPermissionsLinux
    fileChmod
    folderChmod
    chownGroup`;

const libraryPathsFieldSelection = `
    moviePath
    seriesPath
    animePath`;

const serviceSettingsFieldSelection = `
    tlsCertPath
    tlsKeyPath`;

export const saveQualityProfileSettingsMutation = `mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
  saveQualityProfileSettings(input: $input) {${qualityProfileSettingsFieldSelection}
  }
}`;

export const deleteQualityProfileMutation = `mutation DeleteQualityProfile($id: ID!) {
  deleteQualityProfile(id: $id) {
${qualityProfileSettingsFieldSelection}
  }
}`;

export const updateDownloadClientRoutingMutation = `mutation UpdateDownloadClientRouting($input: UpdateDownloadClientRoutingInput!) {
  updateDownloadClientRouting(input: $input) {${downloadClientRoutingFieldSelection}
  }
}`;

export const updateIndexerRoutingMutation = `mutation UpdateIndexerRouting($input: UpdateIndexerRoutingInput!) {
  updateIndexerRouting(input: $input) {${indexerRoutingFieldSelection}
  }
}`;

export const updateMediaSettingsMutation = `mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
  updateMediaSettings(input: $input) {${mediaSettingsFieldSelection}
  }
}`;

export const updateLibraryPathsMutation = `mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
  updateLibraryPaths(input: $input) {${libraryPathsFieldSelection}
  }
}`;

export const updateServiceSettingsMutation = `mutation UpdateServiceSettings($input: UpdateServiceSettingsInput!) {
  updateServiceSettings(input: $input) {${serviceSettingsFieldSelection}
  }
}`;

export const queueExistingMutation = `mutation QueueExisting($input: QueueDownloadInput!) {
  queueExistingTitleDownload(input: $input) {
    status
    jobId
    titleId
    titleName
    sourceTitle
    sourceKind
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        kind
        episodeId
        episodeIds
        collectionId
      }
    }
  }
}`;

export const triggerTitleMismatchRecoverySearchMutation = `mutation TriggerTitleMismatchRecoverySearch($titleId: ID!) {
  triggerTitleMismatchRecoverySearch(titleId: $titleId) {
    titleId
    queuedCount
  }
}`;

export const queueBestReleaseMutation = `mutation QueueBestRelease($input: QueueBestReleaseInput!) {
  queueBestRelease(input: $input) {
    status
    jobId
    titleId
    titleName
    sourceTitle
    sourceKind
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        kind
        episodeId
        episodeIds
        collectionId
      }
    }
  }
}`;

export const queueManualImportMutation = `mutation QueueManualImport($input: QueueManualImportInput!) {
  queueManualImport(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    importId
    removed
    queueItem {
      id
      titleId
      titleName
      clientId
      clientType
      downloadClientItemId
      state
      importStatus
      importErrorCode
      importErrorMessage
      trackedState
    }
  }
}`;

export const queuePathManualImportMutation = `mutation QueuePathManualImport($input: QueuePathManualImportInput!) {
  queuePathManualImport(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    importId
    removed
  }
}`;

export const pauseDownloadMutation = `mutation PauseDownload($input: PauseDownloadInput!) {
  pauseDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      clientId
      clientType
      downloadClientItemId
      state
    }
  }
}`;

export const resumeDownloadMutation = `mutation ResumeDownload($input: ResumeDownloadInput!) {
  resumeDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      clientId
      clientType
      downloadClientItemId
      state
    }
  }
}`;

export const deleteDownloadMutation = `mutation DeleteDownload($input: DeleteDownloadInput!) {
  deleteDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    commandId
    removed
    clientType
    queueItem {
      id
      clientId
      clientType
      downloadClientItemId
      state
      deleteStatus
      deleteErrorMessage
    }
  }
}`;

export function buildIgnoreTrackedDownloadBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: IgnoreTrackedDownloadInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) =>
      `item${index}: ignoreTrackedDownload(input: $input${index}) { kind }`,
  ).join("\n");

  return `mutation IgnoreTrackedDownloads(${variables}) {
${fields}
}`;
}

export function buildDeleteDownloadBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: DeleteDownloadInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) =>
      `item${index}: deleteDownload(input: $input${index}) { kind removed commandId }`,
  ).join("\n");

  return `mutation DeleteDownloads(${variables}) {
${fields}
}`;
}

export const setCollectionMonitoredMutation = `mutation SetCollectionMonitored($input: SetCollectionMonitoredInput!) {
  setCollectionMonitored(input: $input) {
    id
    monitored
    episodes {
      id
      titleId
      collectionId
      episodeType
      episodeNumber
      seasonNumber
      episodeLabel
      title
      overview
      airDate
      durationSeconds
      hasMultiAudio
      hasSubtitle
      isFiller
      absoluteNumber
      monitored
      createdAt
    }
  }
}`;

export const setEpisodeMonitoredMutation = `mutation SetEpisodeMonitored($input: SetEpisodeMonitoredInput!) {
  setEpisodeMonitored(input: $input) { id monitored }
}`;

export const setSeriesMovieMonitoredMutation = `mutation SetSeriesMovieMonitored($input: SetSeriesMovieMonitoredInput!) {
  setSeriesMovieMonitored(input: $input) { id monitored }
}`;

export const setTitleMonitoredMutation = `mutation SetTitleMonitored($input: SetTitleMonitoredInput!) {
  setTitleMonitored(input: $input) { id monitored }
}`;

export const updateTitleMutation = `mutation UpdateTitle($input: UpdateTitleInput!) {
  updateTitle(input: $input) {
    id
    name
    facet
    tags
    monitored
    qualityProfileId
    rootFolderId
    rootFolderPath
    monitorType
    useSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
  }
}`;

export function buildSetTitleMonitoredBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: SetTitleMonitoredInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) =>
      `item${index}: setTitleMonitored(input: $input${index}) { id monitored }`,
  ).join("\n");

  return `mutation SetTitleMonitoredBatch(${variables}) {
${fields}
}`;
}

export function buildUpdateTitleBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: UpdateTitleInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) => `item${index}: updateTitle(input: $input${index}) { id }`,
  ).join("\n");

  return `mutation UpdateTitleBatch(${variables}) {
${fields}
}`;
}

export function buildDeleteTitleBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: DeleteTitleInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) => `item${index}: deleteTitle(input: $input${index}) {
    id
    deleted
  }`,
  ).join("\n");

  return `mutation DeleteTitleBatch(${variables}) {
${fields}
}`;
}

export const fixTitleMatchMutation = `mutation FixTitleMatch($input: FixTitleMatchInput!) {
  fixTitleMatch(input: $input) {
    hydrated
    warnings
    libraryScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    title {
      id
      name
      facet
      externalIds {
        source
        value
      }
      imdbId
      slug
      metadataFetchedAt
    }
  }
}`;

export const setPrimaryMovieFileMutation = `mutation SetPrimaryMovieFile($input: SetPrimaryMovieFileInput!) {
  setPrimaryMovieFile(input: $input) {
    id
  }
}`;

const wantedSearchPayloadSelection = `
    queuedCount
    skippedInProgressCount
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        kind
        episodeId
        episodeIds
        collectionId
      }
    }`;

export const triggerWantedSearchMutation = `mutation TriggerWantedSearch($input: TriggerWantedSearchInput!) {
  triggerWantedSearch(input: $input) {${wantedSearchPayloadSelection}
  }
}`;

export const triggerTitleWantedSearchMutation = `mutation TriggerTitleWantedSearch($input: TriggerTitleWantedSearchInput!) {
  triggerTitleWantedSearch(input: $input) {${wantedSearchPayloadSelection}
  }
}`;

export const triggerSeasonWantedSearchMutation = `mutation TriggerSeasonWantedSearch($input: TriggerSeasonWantedSearchInput!) {
  triggerSeasonWantedSearch(input: $input) {${wantedSearchPayloadSelection}
  }
}`;

export const pauseWantedItemMutation = `mutation PauseWantedItem($id: ID!) {
  pauseWantedItem(id: $id) {
    id
    paused
  }
}`;

export const resumeWantedItemMutation = `mutation ResumeWantedItem($id: ID!) {
  resumeWantedItem(id: $id) {
    id
    resumed
  }
}`;

export const resetWantedItemMutation = `mutation ResetWantedItem($id: ID!) {
  resetWantedItem(id: $id) {
    id
    reset
  }
}`;

// ── RSS Sync ─────────────────────────────────────────────────────────────

export const triggerRssSyncMutation = `mutation TriggerRssSync {
  triggerRssSync {
    releasesFetched
    releasesMatched
    releasesGrabbed
    releasesHeld
  }
}`;

// ── Pending Releases ─────────────────────────────────────────────────────

export const forceGrabPendingReleaseMutation = `mutation ForceGrabPendingRelease($id: ID!) {
  forceGrabPendingRelease(id: $id) {
    id
    grabbed
  }
}`;

export const dismissPendingReleaseMutation = `mutation DismissPendingRelease($id: ID!) {
  dismissPendingRelease(id: $id) {
    id
    dismissed
  }
}`;

// ── Plugins ──────────────────────────────────────────────────────────────

export const refreshPluginCatalogMutation = `mutation RefreshPluginCatalog {
  refreshPluginCatalog {
    id
    name
    description
    version
    latestVersion
    pluginType
    providerType
    author
    official
    publisher
    supportTier
    status
    docsUrl
    sourceRepo
    builtin
    sourceUrl
    sourceKind
    blockedReason
    bytes
    isInstalled
    isEnabled
    installedVersion
    updateAvailable
    installInProgress
    defaultBaseUrl
  }
}`;

export const beginInstallPluginMutation = `mutation BeginInstallPlugin($pluginId: ID!) {
  beginInstallPlugin(pluginId: $pluginId) {
    pluginId
    operationKind
    state
    label
    stepIndex
    stepCount
    message
    error
  }
}`;

export const uninstallPluginMutation = `mutation UninstallPlugin($pluginId: ID!) {
  uninstallPlugin(pluginId: $pluginId) {
    pluginId
    uninstalled
  }
}`;

export const togglePluginMutation = `mutation TogglePlugin($input: TogglePluginInput!) {
  togglePlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

export const beginUpgradePluginMutation = `mutation BeginUpgradePlugin($pluginId: ID!) {
  beginUpgradePlugin(pluginId: $pluginId) {
    pluginId
    operationKind
    state
    label
    stepIndex
    stepCount
    message
    error
  }
}`;

export const inspectManualPluginRepoMutation = `mutation InspectManualPluginRepo($input: ManualPluginRepoInput!) {
  inspectManualPluginRepo(input: $input) {
    githubRepoUrl
    plugin {
      id
      name
      description
      version
      latestVersion
      pluginType
      providerType
      author
      official
      publisher
      supportTier
      status
      docsUrl
      sourceRepo
      builtin
      sourceUrl
      sourceKind
      blockedReason
      bytes
      isInstalled
      isEnabled
      installedVersion
      updateAvailable
      installInProgress
      defaultBaseUrl
    }
  }
}`;

export const installManualPluginMutation = `mutation InstallManualPlugin($input: ManualPluginRepoInput!) {
  installManualPlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

export const installUploadedPluginMutation = `mutation InstallUploadedPlugin($input: ManualPluginUploadInput!) {
  installUploadedPlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

// ── Recycle Bin ─────────────────────────────────────────────────────────

export const restoreRecycledItemMutation = `mutation RestoreRecycledItem($id: ID!) {
  restoreRecycledItem(id: $id) {
    id
    restored
  }
}`;

export const deleteRecycledItemMutation = `mutation DeleteRecycledItem($id: ID!) {
  deleteRecycledItem(id: $id) {
    id
    deleted
  }
}`;

export const emptyRecycleBinMutation = `mutation EmptyRecycleBin($libraryIds: [ID!]) {
  emptyRecycleBin(libraryIds: $libraryIds) {
    purgedCount
  }
}`;

export const updateRecycleBinSettingsMutation = `mutation UpdateRecycleBinSettings($input: UpdateRecycleBinSettingsInput!) {
  updateRecycleBinSettings(input: $input) {
    enabled
  }
}`;

// ── Notifications ────────────────────────────────────────────────────────

export const createNotificationChannelMutation = `mutation CreateNotificationChannel($input: CreateNotificationChannelInput!) {
  createNotificationChannel(input: $input) {
    id
    name
    channelType
    mediaServerConnectionId
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const updateNotificationChannelMutation = `mutation UpdateNotificationChannel($input: UpdateNotificationChannelInput!) {
  updateNotificationChannel(input: $input) {
    id
    name
    channelType
    mediaServerConnectionId
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const deleteNotificationChannelMutation = `mutation DeleteNotificationChannel($id: ID!) {
  deleteNotificationChannel(id: $id) {
    id
    deleted
  }
}`;

export const testNotificationChannelMutation = `mutation TestNotificationChannel($id: ID!) {
  testNotificationChannel(id: $id) {
    id
    status
    message
    retryAfterSeconds
  }
}`;

export const createNotificationSubscriptionMutation = `mutation CreateNotificationSubscription($input: CreateNotificationSubscriptionInput!) {
  createNotificationSubscription(input: $input) {
    id
    channelId
    targetKind
    targetId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const updateNotificationSubscriptionMutation = `mutation UpdateNotificationSubscription($input: UpdateNotificationSubscriptionInput!) {
  updateNotificationSubscription(input: $input) {
    id
    channelId
    targetKind
    targetId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const deleteNotificationSubscriptionMutation = `mutation DeleteNotificationSubscription($id: ID!) {
  deleteNotificationSubscription(id: $id) {
    id
    deleted
  }
}`;

// ── Rule Sets ────────────────────────────────────────────────────────────

export const createRuleSetMutation = `mutation CreateRuleSet($input: CreateRuleSetInput!) {
  createRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const updateRuleSetMutation = `mutation UpdateRuleSet($input: UpdateRuleSetInput!) {
  updateRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const deleteRuleSetMutation = `mutation DeleteRuleSet($id: ID!) {
  deleteRuleSet(id: $id) {
    id
    deleted
  }
}`;

export const toggleRuleSetMutation = `mutation ToggleRuleSet($input: ToggleRuleSetInput!) {
  toggleRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const validateRuleSetMutation = `mutation ValidateRuleSet($input: ValidateRuleSetInput!) {
  validateRuleSet(input: $input) {
    valid
    errors
  }
}`;

export const setTitleRequiredAudioMutation = `mutation SetTitleRequiredAudio($input: SetTitleRequiredAudioInput!) {
  setTitleRequiredAudio(input: $input) {
    titleId
    facet
    languages
    updated
  }
}`;

// ── Setup Wizard ──────────────────────────────────────────────────────

export const completeSetupMutation = `mutation CompleteSetup {
  completeSetup {
    completed
  }
}`;

// ── External Import (Sonarr/Radarr) ──────────────────────────────────

const EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS = `
    sessionId
    status
    phase
    startedAt
    updatedAt
    overallTotalKnown
    overallProgress { total completed failed }
    moviesTotalKnown
    moviesProgress { total completed failed }
    seriesTotalKnown
    seriesProgress { total completed failed }
    episodeFetchTotalKnown
    episodeFetchExpectedTotal
    episodeFetchExpectedMonitoredTotal
    episodeFetchProgress { total completed failed }
    snapshotBuildTotalKnown
    snapshotBuildProgress { total completed failed }
    matchedMovieCount
    matchedSeriesCount
    unmatchedMovieCount
    unmatchedSeriesCount
    ambiguousMovieCount
    ambiguousSeriesCount
    errorMessage
`;

export const previewExternalImportMutation = `mutation PreviewExternalImport($input: PreviewExternalImportInput!) {
  previewExternalImport(input: $input) {
    sonarrConnected
    radarrConnected
    prowlarrConnected
    sonarrVersion
    radarrVersion
    prowlarrVersion
    sonarrError
    radarrError
    prowlarrError
    rootFolders { source path }
    downloadClients {
      sources name implementation scryerClientType
      host port useSsl urlBase username apiKeyPresent
      dedupKey supported requiresPasswordOverride
    }
    indexers {
      sources name implementation scryerProviderType
      baseUrl apiKeyPresent dedupKey supported
      childCount childNames requiresApiKeyOverride apiKeyHelpUrl
    }
  }
}`;

export const startExternalImportMonitorWarmupMutation = `mutation StartExternalImportMonitorWarmup($input: StartExternalImportMonitorWarmupInput!) {
  startExternalImportMonitorWarmup(input: $input) {${EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS}
  }
}`;

export const cancelExternalImportMonitorWarmupMutation = `mutation CancelExternalImportMonitorWarmup($sessionId: ID!) {
  cancelExternalImportMonitorWarmup(sessionId: $sessionId) {
    sessionId
    canceled
  }
}`;

export const executeExternalImportMutation = `mutation ExecuteExternalImport($input: ExecuteExternalImportInput!) {
  executeExternalImport(input: $input) {
    mediaPathsSaved
    downloadClientsCreated
    indexersCreated
    pluginsInstalled
    errors
  }
}`;

export const finalizeExternalImportMutation = `mutation FinalizeExternalImport($input: FinalizeExternalImportInput!) {
  finalizeExternalImport(input: $input) {
    finalized
    monitorWarmupSessionId
  }
}`;

export const rehydrateAllMetadataMutation = `mutation RehydrateAllMetadata($input: RehydrateAllMetadataInput!) {
  rehydrateAllMetadata(input: $input) {
    language
    titlesCleared
    accepted
  }
}`;

const ppScriptFields = `
    id name description scriptType scriptContent appliedFacets
    executionMode timeoutSecs priority enabled debug createdAt updatedAt
`;

export const createPostProcessingScriptMutation = `mutation CreatePostProcessingScript($input: CreatePostProcessingScriptInput!) {
  createPostProcessingScript(input: $input) {${ppScriptFields}}
}`;

export const updatePostProcessingScriptMutation = `mutation UpdatePostProcessingScript($input: UpdatePostProcessingScriptInput!) {
  updatePostProcessingScript(input: $input) {${ppScriptFields}}
}`;

export const deletePostProcessingScriptMutation = `mutation DeletePostProcessingScript($id: ID!) {
  deletePostProcessingScript(id: $id) {
    id
    deleted
  }
}`;

export const togglePostProcessingScriptMutation = `mutation TogglePostProcessingScript($id: ID!, $inlineShellAcknowledged: Boolean) {
  togglePostProcessingScript(id: $id, inlineShellAcknowledged: $inlineShellAcknowledged) {${ppScriptFields}}
}`;

// Input type companion — keep in sync with ExecuteExternalImportInput on the backend.
export type DownloadClientApiKeyOverride = {
  dedupKey: string;
  apiKey: string;
};

export type DownloadClientPasswordOverride = {
  dedupKey: string;
  password: string;
};

export type IndexerApiKeyOverride = {
  dedupKey: string;
  apiKey: string;
};

// ── Subtitle mutations ──────────────────────────────────────────────────────

export const searchSubtitlesMutation = `mutation SearchSubtitles($input: SearchSubtitlesInput!) {
  searchSubtitles(input: $input) {
    provider
    providerFileId
    language
    releaseInfo
    score
    hearingImpaired
    forced
    aiTranslated
    machineTranslated
    uploader
    downloadCount
    hashMatched
  }
}`;

export const downloadSubtitleMutation = `mutation DownloadSubtitle($input: DownloadSubtitleInput!) {
  downloadSubtitle(input: $input) {
    mediaFileId
    providerFileId
    downloaded
  }
}`;

export const deleteExternalSubtitleMutation = `mutation DeleteExternalSubtitle($input: DeleteExternalSubtitleInput!) {
  deleteExternalSubtitle(input: $input) {
    id
    deleted
  }
}`;

export const blocklistExternalSubtitleMutation = `mutation BlocklistExternalSubtitle($input: BlocklistExternalSubtitleInput!) {
  blocklistExternalSubtitle(input: $input) {
    id
    blocklisted
  }
}`;

export const clearTitleReleaseBlocklistEntryMutation = `mutation ClearTitleReleaseBlocklistEntry($id: ID!) {
  clearTitleReleaseBlocklistEntry(id: $id) {
    id
    cleared
  }
}`;

// ── Import retry mutations ────────────────────────────────────────────────

export const retryImportMutation = `mutation RetryImport($input: RetryImportInput!) {
  retryImport(input: $input) {
    importId
    decision
    skipReason
    titleId
    sourcePath
    destPath
    errorMessage
  }
}`;

export const ignoreTrackedDownloadMutation = `mutation IgnoreTrackedDownload($input: IgnoreTrackedDownloadInput!) {
  ignoreTrackedDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      clientId
      clientType
      downloadClientItemId
      state
      trackedState
      trackedStatus
    }
  }
}`;

export const markTrackedDownloadFailedMutation = `mutation MarkTrackedDownloadFailed($input: MarkTrackedDownloadFailedInput!) {
  markTrackedDownloadFailed(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      clientId
      clientType
      downloadClientItemId
      state
      trackedState
      trackedStatus
    }
  }
}`;

export const assignTrackedDownloadTitleMutation = `mutation AssignTrackedDownloadTitle($input: AssignTrackedDownloadTitleInput!) {
  assignTrackedDownloadTitle(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      facet
      clientId
      clientType
      downloadClientItemId
      state
      trackedState
      trackedStatus
    }
  }
}`;

export type SubtitleSearchResult = {
  provider: string;
  providerFileId: string;
  language: string;
  releaseInfo: string | null;
  score: number;
  hearingImpaired: boolean;
  forced: boolean;
  aiTranslated: boolean;
  machineTranslated: boolean;
  uploader: string | null;
  downloadCount: number | null;
  hashMatched: boolean;
};
