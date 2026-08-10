
    document.addEventListener("DOMContentLoaded", () => {
        const messageElement = document.querySelector('.message');
        vegordSplashNative.onUpdateMessage(message => {
            messageElement.textContent = message;
        });

        document.querySelector('#donate-btn').addEventListener('click', () => {
            vegordSplashNative.openExternal('https://vergoboy.ir/donate');
        });

        document.querySelector('#github-btn').addEventListener('click', () => {
            vegordSplashNative.openExternal('https://github.com/vergoboy/vegord');
        });
    });
