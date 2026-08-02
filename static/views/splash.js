
    document.addEventListener("DOMContentLoaded", () => {
        const messageElement = document.querySelector('.message');
        VegcordSplashNative.onUpdateMessage(message => {
            messageElement.textContent = message;
        });

        document.querySelector('#donate-btn').addEventListener('click', () => {
            VegcordSplashNative.openExternal('https://vergoboy.ir/donate');
        });

        document.querySelector('#github-btn').addEventListener('click', () => {
            VegcordSplashNative.openExternal('https://github.com/vergoboy/vegord');
        });
    });
