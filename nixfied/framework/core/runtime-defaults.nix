{
  hosts = {
    loopbackIp = "127.0.0.1";
    localhost = "localhost";
  };

  intervals = {
    pollFastSeconds = "0.1";
    pollDefaultSeconds = "1";
    pollSlowSeconds = "2";
    retryDefaultSeconds = "5";
  };

  probes = {
    wait = {
      enabled = false;
      timeoutSeconds = 300;
      intervalSeconds = 1;
      timeoutEnvVar = null;
      intervalEnvVar = null;
    };

    httpMaxTimeSeconds = 2;

    startupReadiness = {
      attempts = 40;
      extendedAttempts = 80;
      intervalSeconds = "0.25";
      tailLines = 50;
    };

    managedStop = {
      waitAttempts = 20;
      extendedWaitAttempts = 40;
      waitIntervalSeconds = "0.2";
      extendedWaitIntervalSeconds = "0.25";
    };
  };
}
